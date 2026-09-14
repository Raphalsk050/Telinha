#include "telinha/app/machine_events.hpp"

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <new>
#include <thread>
#include <utility>

#include "telinha/app/signaling.hpp"

namespace tl::app {
namespace {

constexpr std::size_t kCommandQueueCapacity = 64;
constexpr std::size_t kCommandLineBytes = kTokenCapacity + 1024;
constexpr std::size_t kKeyCapacity = 64;

struct QueuedCommand {
    MachineCommand command;
    std::unique_ptr<char[]> payload;
};

std::atomic<bool> g_enabled{false};
std::mutex g_output_mutex;

std::mutex g_command_mutex;
QueuedCommand g_commands[kCommandQueueCapacity];
std::unique_ptr<char[]> g_taken_payload;
std::size_t g_command_head = 0;
std::size_t g_command_count = 0;

void write_string(const char* value, std::size_t length) noexcept
{
    std::fputc('"', stdout);
    for (std::size_t index = 0; index < length; ++index) {
        const auto symbol = static_cast<unsigned char>(value[index]);
        if (symbol == '"' || symbol == '\\') {
            std::fputc('\\', stdout);
            std::fputc(symbol, stdout);
        } else if (symbol == '\n') {
            std::fputs("\\n", stdout);
        } else if (symbol == '\r') {
            std::fputs("\\r", stdout);
        } else if (symbol == '\t') {
            std::fputs("\\t", stdout);
        } else if (symbol < 0x20 || symbol == 0x7F) {
            std::fprintf(stdout, "\\u%04x", static_cast<unsigned>(symbol));
        } else {
            std::fputc(symbol, stdout);
        }
    }
    std::fputc('"', stdout);
}

const char* skip_space(const char* cursor) noexcept
{
    while (*cursor == ' ' || *cursor == '\t' || *cursor == '\r' || *cursor == '\n') {
        ++cursor;
    }
    return cursor;
}

const char* string_end(const char* quote) noexcept
{
    for (const char* cursor = quote + 1; *cursor != '\0'; ++cursor) {
        if (*cursor == '\\') {
            if (cursor[1] == '\0') {
                return nullptr;
            }
            ++cursor;
        } else if (*cursor == '"') {
            return cursor;
        }
    }
    return nullptr;
}

const char* value_end(const char* value) noexcept
{
    if (*value == '"') {
        const char* end = string_end(value);
        return end == nullptr ? nullptr : end + 1;
    }

    if (*value == '{' || *value == '[') {
        int depth = 0;
        for (const char* cursor = value; *cursor != '\0'; ++cursor) {
            if (*cursor == '"') {
                cursor = string_end(cursor);
                if (cursor == nullptr) {
                    return nullptr;
                }
            } else if (*cursor == '{' || *cursor == '[') {
                ++depth;
            } else if (*cursor == '}' || *cursor == ']') {
                --depth;
                if (depth == 0) {
                    return cursor + 1;
                }
            }
        }
        return nullptr;
    }

    const char* cursor = value;
    while (*cursor != '\0' && *cursor != ',' && *cursor != '}' && *cursor != ']' &&
           *cursor != ' ' && *cursor != '\t' && *cursor != '\r' && *cursor != '\n') {
        ++cursor;
    }
    return cursor == value ? nullptr : cursor;
}

bool read_hex_unit(const char* text, std::uint32_t& out) noexcept
{
    std::uint32_t value = 0;
    for (int index = 0; index < 4; ++index) {
        const char symbol = text[index];
        std::uint32_t digit = 0;
        if (symbol >= '0' && symbol <= '9') {
            digit = static_cast<std::uint32_t>(symbol - '0');
        } else if (symbol >= 'a' && symbol <= 'f') {
            digit = static_cast<std::uint32_t>(symbol - 'a') + 10u;
        } else if (symbol >= 'A' && symbol <= 'F') {
            digit = static_cast<std::uint32_t>(symbol - 'A') + 10u;
        } else {
            return false;
        }
        value = (value << 4) | digit;
    }
    out = value;
    return true;
}

bool append_utf8(std::uint32_t code_point, char* out, std::size_t capacity,
                 std::size_t& written) noexcept
{
    char bytes[4];
    std::size_t count = 0;
    if (code_point < 0x80u) {
        bytes[0] = static_cast<char>(code_point);
        count = 1;
    } else if (code_point < 0x800u) {
        bytes[0] = static_cast<char>(0xC0u | (code_point >> 6));
        bytes[1] = static_cast<char>(0x80u | (code_point & 0x3Fu));
        count = 2;
    } else if (code_point < 0x10000u) {
        bytes[0] = static_cast<char>(0xE0u | (code_point >> 12));
        bytes[1] = static_cast<char>(0x80u | ((code_point >> 6) & 0x3Fu));
        bytes[2] = static_cast<char>(0x80u | (code_point & 0x3Fu));
        count = 3;
    } else {
        bytes[0] = static_cast<char>(0xF0u | (code_point >> 18));
        bytes[1] = static_cast<char>(0x80u | ((code_point >> 12) & 0x3Fu));
        bytes[2] = static_cast<char>(0x80u | ((code_point >> 6) & 0x3Fu));
        bytes[3] = static_cast<char>(0x80u | (code_point & 0x3Fu));
        count = 4;
    }
    if (capacity - written < count) {
        return false;
    }
    std::memcpy(out + written, bytes, count);
    written += count;
    return true;
}

bool decode_string(const char* quote, char* out, std::size_t capacity, std::size_t& length) noexcept
{
    std::size_t written = 0;
    const char* cursor = quote + 1;
    for (;;) {
        const char symbol = *cursor;
        if (symbol == '\0') {
            return false;
        }
        if (symbol == '"') {
            break;
        }
        if (symbol != '\\') {
            if (written == capacity) {
                return false;
            }
            out[written++] = symbol;
            ++cursor;
            continue;
        }

        const char escape = cursor[1];
        if (escape == 'u') {
            std::uint32_t unit = 0;
            if (!read_hex_unit(cursor + 2, unit)) {
                return false;
            }
            cursor += 6;

            std::uint32_t code_point = unit;
            if (unit >= 0xD800u && unit <= 0xDBFFu) {
                std::uint32_t low = 0;
                if (cursor[0] == '\\' && cursor[1] == 'u' && read_hex_unit(cursor + 2, low) &&
                    low >= 0xDC00u && low <= 0xDFFFu) {
                    code_point = 0x10000u + ((unit - 0xD800u) << 10) + (low - 0xDC00u);
                    cursor += 6;
                } else {
                    code_point = 0xFFFDu;
                }
            } else if (unit >= 0xDC00u && unit <= 0xDFFFu) {
                code_point = 0xFFFDu;
            }

            if (code_point == 0 || !append_utf8(code_point, out, capacity, written)) {
                return false;
            }
            continue;
        }

        char decoded = 0;
        switch (escape) {
            case '"': decoded = '"'; break;
            case '\\': decoded = '\\'; break;
            case '/': decoded = '/'; break;
            case 'b': decoded = '\b'; break;
            case 'f': decoded = '\f'; break;
            case 'n': decoded = '\n'; break;
            case 'r': decoded = '\r'; break;
            case 't': decoded = '\t'; break;
            default: return false;
        }
        if (written == capacity) {
            return false;
        }
        out[written++] = decoded;
        cursor += 2;
    }
    length = written;
    return true;
}

const char* find_value(const char* line, const char* key) noexcept
{
    const std::size_t key_length = std::strlen(key);
    const char* cursor = skip_space(line);
    if (*cursor != '{') {
        return nullptr;
    }
    cursor = skip_space(cursor + 1);

    while (*cursor == '"') {
        const char* key_end = string_end(cursor);
        if (key_end == nullptr) {
            return nullptr;
        }

        char name[kKeyCapacity];
        std::size_t name_length = 0;
        const bool matches = decode_string(cursor, name, sizeof(name), name_length) &&
                             name_length == key_length && std::memcmp(name, key, key_length) == 0;

        cursor = skip_space(key_end + 1);
        if (*cursor != ':') {
            return nullptr;
        }
        cursor = skip_space(cursor + 1);
        if (matches) {
            return cursor;
        }

        cursor = value_end(cursor);
        if (cursor == nullptr) {
            return nullptr;
        }
        cursor = skip_space(cursor);
        if (*cursor != ',') {
            return nullptr;
        }
        cursor = skip_space(cursor + 1);
    }
    return nullptr;
}

bool read_text(const char* line, const char* key, char* out, std::size_t capacity) noexcept
{
    const char* value = find_value(line, key);
    std::size_t length = 0;
    if (value == nullptr || *value != '"' || capacity == 0 ||
        !decode_string(value, out, capacity - 1, length)) {
        return false;
    }
    out[length] = '\0';
    return true;
}

bool read_payload(const char* line, const char* key, Span<char> scratch,
                  MachineCommand& out) noexcept
{
    const char* value = find_value(line, key);
    std::size_t length = 0;
    if (value == nullptr || *value != '"' ||
        !decode_string(value, scratch.data(), scratch.size(), length)) {
        return false;
    }
    out.payload = scratch.data();
    out.payload_length = length;
    return true;
}

bool read_peer(const char* line, char* out) noexcept
{
    if (!read_text(line, "peer", out, kPeerIdCapacity) || out[0] == '\0') {
        return false;
    }
    for (const char* cursor = out; *cursor != '\0'; ++cursor) {
        const char symbol = *cursor;
        const bool allowed = (symbol >= 'A' && symbol <= 'Z') || (symbol >= 'a' && symbol <= 'z') ||
                             (symbol >= '0' && symbol <= '9') || symbol == '_' || symbol == '-';
        if (!allowed) {
            return false;
        }
    }
    return true;
}

bool read_unsigned(const char* line, const char* key, std::uint64_t& out) noexcept
{
    const char* value = find_value(line, key);
    if (value == nullptr || *value < '0' || *value > '9') {
        return false;
    }
    char* end = nullptr;
    out = std::strtoull(value, &end, 10);
    return end != value;
}

std::uint32_t read_limit(const char* line, const char* key) noexcept
{
    std::uint64_t value = 0;
    if (!read_unsigned(line, key, value)) {
        return 0;
    }
    return value > 0xFFFFFFFFull ? 0xFFFFFFFFu : static_cast<std::uint32_t>(value);
}

void push_command(const MachineCommand& command) noexcept
{
    MachineCommand stored = command;
    stored.payload = nullptr;
    stored.payload_length = 0;

    std::unique_ptr<char[]> payload;
    if (command.payload != nullptr && command.payload_length != 0) {
        payload.reset(new (std::nothrow) char[command.payload_length]);
        if (payload) {
            std::memcpy(payload.get(), command.payload, command.payload_length);
            stored.payload = payload.get();
            stored.payload_length = command.payload_length;
        }
    }

    const std::lock_guard<std::mutex> guard(g_command_mutex);
    if (g_command_count == kCommandQueueCapacity) {
        g_command_head = (g_command_head + 1) % kCommandQueueCapacity;
        --g_command_count;
    }
    QueuedCommand& slot = g_commands[(g_command_head + g_command_count) % kCommandQueueCapacity];
    slot.command = stored;
    slot.payload = std::move(payload);
    ++g_command_count;
}

}  // namespace

void enable_machine_events() noexcept
{
    g_enabled.store(true, std::memory_order_relaxed);
}

bool machine_events_enabled() noexcept
{
    return g_enabled.load(std::memory_order_relaxed);
}

MachineEvent::MachineEvent(const char* name) noexcept : lock_(g_output_mutex)
{
    std::fputs("{\"event\":", stdout);
    write_string(name, std::strlen(name));
    needs_comma_[0] = true;
}

MachineEvent::~MachineEvent()
{
    while (depth_ > 0) {
        std::fputc(']', stdout);
        --depth_;
    }
    std::fputs("}\n", stdout);
    std::fflush(stdout);
}

void MachineEvent::separate() noexcept
{
    if (needs_comma_[depth_]) {
        std::fputc(',', stdout);
    }
    needs_comma_[depth_] = true;
}

void MachineEvent::write_key(const char* key) noexcept
{
    separate();
    write_string(key, std::strlen(key));
    std::fputc(':', stdout);
}

MachineEvent& MachineEvent::text(const char* key, const char* value) noexcept
{
    return text(key, value, value == nullptr ? 0 : std::strlen(value));
}

MachineEvent& MachineEvent::text(const char* key, const char* value, std::size_t length) noexcept
{
    write_key(key);
    write_string(value == nullptr ? "" : value, value == nullptr ? 0 : length);
    return *this;
}

MachineEvent& MachineEvent::integer(const char* key, std::uint64_t value) noexcept
{
    write_key(key);
    std::fprintf(stdout, "%llu", static_cast<unsigned long long>(value));
    return *this;
}

MachineEvent& MachineEvent::number(const char* key, double value) noexcept
{
    write_key(key);
    if (std::isfinite(value)) {
        std::fprintf(stdout, "%.3f", value);
    } else {
        std::fputs("null", stdout);
    }
    return *this;
}

MachineEvent& MachineEvent::flag(const char* key, bool value) noexcept
{
    write_key(key);
    std::fputs(value ? "true" : "false", stdout);
    return *this;
}

MachineEvent& MachineEvent::begin_array(const char* key) noexcept
{
    if (depth_ + 1 >= kMaxDepth) {
        return *this;
    }
    write_key(key);
    std::fputc('[', stdout);
    ++depth_;
    needs_comma_[depth_] = false;
    return *this;
}

MachineEvent& MachineEvent::end_array() noexcept
{
    if (depth_ > 0) {
        std::fputc(']', stdout);
        --depth_;
    }
    return *this;
}

MachineEvent& MachineEvent::begin_object() noexcept
{
    if (depth_ + 1 >= kMaxDepth) {
        return *this;
    }
    separate();
    std::fputc('{', stdout);
    ++depth_;
    needs_comma_[depth_] = false;
    return *this;
}

MachineEvent& MachineEvent::end_object() noexcept
{
    if (depth_ > 0) {
        std::fputc('}', stdout);
        --depth_;
    }
    return *this;
}

bool parse_machine_command(const char* line, MachineCommand& out, Span<char> scratch) noexcept
{
    out = MachineCommand{};

    char name[32];
    if (!read_text(line, "command", name, sizeof(name))) {
        return false;
    }

    if (std::strcmp(name, "stop") == 0) {
        out.kind = MachineCommandKind::Stop;
        return true;
    }

    if (std::strcmp(name, "switch_target") == 0) {
        char kind[16];
        std::uint64_t handle = 0;
        if (!read_text(line, "kind", kind, sizeof(kind)) ||
            !read_unsigned(line, "handle", handle) || handle == 0) {
            return false;
        }
        if (std::strcmp(kind, "window") == 0) {
            out.window = true;
        } else if (std::strcmp(kind, "device") == 0) {
            out.device = true;
        } else if (std::strcmp(kind, "monitor") != 0) {
            return false;
        }
        out.kind = MachineCommandKind::SwitchTarget;
        out.handle = handle;
        return true;
    }

    if (std::strcmp(name, "set_quality") == 0) {
        out.kind = MachineCommandKind::SetQuality;
        out.max_fps = read_limit(line, "max_fps");
        out.max_width = read_limit(line, "max_width");
        out.max_height = read_limit(line, "max_height");
        out.max_bitrate_kbps = read_limit(line, "max_bitrate_kbps");
        return true;
    }

    if (std::strcmp(name, "set_fullscreen") == 0) {
        const char* value = find_value(line, "enabled");
        if (value == nullptr) {
            return false;
        }
        out.kind = MachineCommandKind::SetFullscreen;
        out.enabled = std::strncmp(value, "true", 4) == 0;
        return true;
    }

    if (std::strcmp(name, "set_audio") == 0) {
        if (!read_text(line, "scope", out.scope, sizeof(out.scope))) {
            return false;
        }
        const char* device = find_value(line, "device");
        if (device != nullptr && *device == '"' &&
            !read_text(line, "device", out.device_id, sizeof(out.device_id))) {
            return false;
        }
        out.kind = MachineCommandKind::SetAudio;
        out.pid = read_limit(line, "pid");
        return true;
    }

    if (std::strcmp(name, "set_volume") == 0) {
        std::uint64_t volume = 0;
        if (!read_unsigned(line, "volume", volume)) {
            return false;
        }
        out.kind = MachineCommandKind::SetVolume;
        out.volume = volume > 0xFFFFFFFFull ? 0xFFFFFFFFu : static_cast<std::uint32_t>(volume);
        return true;
    }

    if (std::strcmp(name, "add_peer") == 0) {
        if (!read_peer(line, out.peer)) {
            return false;
        }
        const char* format = find_value(line, "format");
        if (format != nullptr && std::strncmp(format, "null", 4) != 0) {
            char value[16];
            if (!read_text(line, "format", value, sizeof(value))) {
                return false;
            }
            if (std::strcmp(value, "sdp") == 0) {
                out.format = PeerFormat::Sdp;
            } else if (std::strcmp(value, "token") != 0) {
                return false;
            }
        }
        out.kind = MachineCommandKind::AddPeer;
        return true;
    }

    if (std::strcmp(name, "remove_peer") == 0) {
        if (!read_peer(line, out.peer)) {
            return false;
        }
        out.kind = MachineCommandKind::RemovePeer;
        return true;
    }

    if (std::strcmp(name, "peer_answer") == 0) {
        if (!read_peer(line, out.peer) || !read_payload(line, "code", scratch, out)) {
            return false;
        }
        out.kind = MachineCommandKind::PeerAnswer;
        return true;
    }

    if (std::strcmp(name, "peer_sdp_answer") == 0) {
        if (!read_peer(line, out.peer) || !read_payload(line, "sdp", scratch, out)) {
            return false;
        }
        out.kind = MachineCommandKind::PeerSdpAnswer;
        return true;
    }

    return false;
}

void start_stop_watcher(std::atomic<bool>& stop)
{
    std::thread([&stop]() noexcept {
        char line[64];
        while (std::fgets(line, static_cast<int>(sizeof(line)), stdin) != nullptr) {
            if (std::strncmp(line, "stop", 4) == 0) {
                break;
            }
        }
        stop.store(true, std::memory_order_relaxed);
    }).detach();
}

void start_command_reader(std::atomic<bool>& stop)
{
    std::thread([&stop]() noexcept {
        const std::unique_ptr<char[]> line(new (std::nothrow) char[kCommandLineBytes]);
        const std::unique_ptr<char[]> scratch(new (std::nothrow) char[kCommandLineBytes]);
        if (!line || !scratch) {
            stop.store(true, std::memory_order_relaxed);
            return;
        }

        bool discarding = false;
        while (std::fgets(line.get(), static_cast<int>(kCommandLineBytes), stdin) != nullptr) {
            const std::size_t length = std::strlen(line.get());
            const bool complete = length + 1 < kCommandLineBytes || line[length - 1] == '\n';
            if (discarding || !complete) {
                discarding = !complete;
                continue;
            }

            if (std::strncmp(line.get(), "stop", 4) == 0) {
                break;
            }

            MachineCommand command;
            if (!parse_machine_command(line.get(), command,
                                       Span<char>(scratch.get(), kCommandLineBytes))) {
                continue;
            }
            if (command.kind == MachineCommandKind::Stop) {
                break;
            }
            push_command(command);
        }
        stop.store(true, std::memory_order_relaxed);
    }).detach();
}

bool take_machine_command(MachineCommand& out) noexcept
{
    const std::lock_guard<std::mutex> guard(g_command_mutex);
    if (g_command_count == 0) {
        return false;
    }
    QueuedCommand& slot = g_commands[g_command_head];
    out = slot.command;
    g_taken_payload = std::move(slot.payload);
    g_command_head = (g_command_head + 1) % kCommandQueueCapacity;
    --g_command_count;
    return true;
}

}  // namespace tl::app
