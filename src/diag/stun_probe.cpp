#include "stun_probe.hpp"

#include <cstring>

#include "report_builder.hpp"

#if defined(_WIN32)
#include <winsock2.h>
#include <ws2tcpip.h>
#else
#include <arpa/inet.h>
#include <netdb.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>
#endif

namespace tl::diag {
namespace {
#if defined(_WIN32)
using SocketHandle = SOCKET;
constexpr SocketHandle kInvalidSocket = INVALID_SOCKET;
#else
using SocketHandle = int;
constexpr SocketHandle kInvalidSocket = -1;
#endif

constexpr std::size_t kStunHeaderBytes = 20;
constexpr std::size_t kTransactionBytes = 12;
constexpr std::uint16_t kBindingRequest = 0x0001;
constexpr std::uint16_t kBindingSuccess = 0x0101;
constexpr std::uint32_t kMagicCookie = 0x2112A442u;
constexpr std::uint16_t kAttrMappedAddress = 0x0001;
constexpr std::uint16_t kAttrXorMappedAddress = 0x0020;
constexpr std::size_t kResponseCapacity = 1024;

void close_socket(SocketHandle handle) noexcept
{
    if (handle == kInvalidSocket) {
        return;
    }
#if defined(_WIN32)
    closesocket(handle);
#else
    ::close(handle);
#endif
}

void write_be16(std::uint8_t* destination, std::uint16_t value) noexcept
{
    destination[0] = static_cast<std::uint8_t>((value >> 8) & 0xFFu);
    destination[1] = static_cast<std::uint8_t>(value & 0xFFu);
}

void write_be32(std::uint8_t* destination, std::uint32_t value) noexcept
{
    destination[0] = static_cast<std::uint8_t>((value >> 24) & 0xFFu);
    destination[1] = static_cast<std::uint8_t>((value >> 16) & 0xFFu);
    destination[2] = static_cast<std::uint8_t>((value >> 8) & 0xFFu);
    destination[3] = static_cast<std::uint8_t>(value & 0xFFu);
}

[[nodiscard]] std::uint16_t read_be16(const std::uint8_t* source) noexcept
{
    return static_cast<std::uint16_t>((static_cast<std::uint16_t>(source[0]) << 8) |
                                      static_cast<std::uint16_t>(source[1]));
}

[[nodiscard]] std::uint32_t read_be32(const std::uint8_t* source) noexcept
{
    return (static_cast<std::uint32_t>(source[0]) << 24) |
           (static_cast<std::uint32_t>(source[1]) << 16) |
           (static_cast<std::uint32_t>(source[2]) << 8) | static_cast<std::uint32_t>(source[3]);
}

void fill_transaction_id(std::uint8_t* destination, std::uint32_t seed) noexcept
{
    std::uint32_t state = seed * 1664525u + 1013904223u;
    for (std::size_t i = 0; i < kTransactionBytes; ++i) {
        state = state * 1664525u + 1013904223u;
        destination[i] = static_cast<std::uint8_t>((state >> 16) & 0xFFu);
    }
}

[[nodiscard]] bool parse_binding_response(const std::uint8_t* data, std::size_t size,
                                          const std::uint8_t* transaction_id,
                                          StunBinding& out) noexcept
{
    if (size < kStunHeaderBytes) {
        return false;
    }

    if (read_be16(data) != kBindingSuccess) {
        return false;
    }

    if (read_be32(data + 4) != kMagicCookie) {
        return false;
    }

    if (std::memcmp(data + 8, transaction_id, kTransactionBytes) != 0) {
        return false;
    }

    const std::size_t declared = read_be16(data + 2);
    const std::size_t available = size - kStunHeaderBytes;
    const std::size_t body = declared < available ? declared : available;

    std::size_t offset = 0;
    while (offset + 4 <= body) {
        const std::uint8_t* attribute = data + kStunHeaderBytes + offset;
        const std::uint16_t type = read_be16(attribute);
        const std::size_t length = read_be16(attribute + 2);
        const std::size_t padded = (length + 3u) & ~std::size_t{3u};

        if (offset + 4 + length > body) {
            return false;
        }

        const bool is_xor = type == kAttrXorMappedAddress;
        if ((is_xor || type == kAttrMappedAddress) && length >= 8) {
            const std::uint8_t family = attribute[5];
            if (family == 0x01) {
                std::uint16_t port = read_be16(attribute + 6);
                std::uint32_t address = read_be32(attribute + 8);
                if (is_xor) {
                    port = static_cast<std::uint16_t>(
                        port ^ static_cast<std::uint16_t>((kMagicCookie >> 16) & 0xFFFFu));
                    address ^= kMagicCookie;
                }
                out.port = port;
                out.address_v4 = address;
                return true;
            }
        }

        offset += 4 + padded;
    }

    return false;
}

[[nodiscard]] bool wait_readable(SocketHandle handle, std::uint32_t timeout_ms) noexcept
{
#if defined(_WIN32)
    fd_set readable;
    FD_ZERO(&readable);
    FD_SET(handle, &readable);

    timeval timeout{};
    timeout.tv_sec = static_cast<long>(timeout_ms / 1000u);
    timeout.tv_usec = static_cast<long>((timeout_ms % 1000u) * 1000u);

    return select(0, &readable, nullptr, nullptr, &timeout) > 0;
#else
    pollfd descriptor{};
    descriptor.fd = handle;
    descriptor.events = POLLIN;
    return poll(&descriptor, 1, static_cast<int>(timeout_ms)) > 0;
#endif
}

[[nodiscard]] bool query_server(SocketHandle handle, const StunServer& server,
                                std::uint32_t timeout_ms, std::uint32_t seed,
                                StunBinding& out) noexcept
{
    addrinfo hints{};
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_DGRAM;

    char port_text[8] = {};
    write_text(port_text, sizeof(port_text), "");
    append_unsigned(port_text, sizeof(port_text), server.port);

    addrinfo* resolved = nullptr;
    if (getaddrinfo(server.host, port_text, &hints, &resolved) != 0 || resolved == nullptr) {
        return false;
    }

    std::uint8_t request[kStunHeaderBytes] = {};
    write_be16(request, kBindingRequest);
    write_be16(request + 2, 0);
    write_be32(request + 4, kMagicCookie);
    fill_transaction_id(request + 8, seed);

    const int sent = static_cast<int>(
        sendto(handle, reinterpret_cast<const char*>(request), static_cast<int>(sizeof(request)), 0,
               resolved->ai_addr, static_cast<int>(resolved->ai_addrlen)));
    freeaddrinfo(resolved);

    if (sent != static_cast<int>(sizeof(request))) {
        return false;
    }

    if (!wait_readable(handle, timeout_ms)) {
        return false;
    }

    std::uint8_t response[kResponseCapacity] = {};
    const int received = static_cast<int>(recvfrom(handle, reinterpret_cast<char*>(response),
                                                   kResponseCapacity, 0, nullptr, nullptr));
    if (received <= 0) {
        return false;
    }

    return parse_binding_response(response, static_cast<std::size_t>(received), request + 8, out);
}

void format_address(char* destination, std::size_t capacity, std::uint32_t address,
                    std::uint16_t port) noexcept
{
    write_text(destination, capacity, "");
    append_unsigned(destination, capacity, (address >> 24) & 0xFFu);
    append_text(destination, capacity, ".");
    append_unsigned(destination, capacity, (address >> 16) & 0xFFu);
    append_text(destination, capacity, ".");
    append_unsigned(destination, capacity, (address >> 8) & 0xFFu);
    append_text(destination, capacity, ".");
    append_unsigned(destination, capacity, address & 0xFFu);
    append_text(destination, capacity, ":");
    append_unsigned(destination, capacity, port);
}

class SocketScope {
public:
    SocketScope() noexcept = default;

    ~SocketScope() { close_socket(handle_); }

    SocketScope(const SocketScope&) = delete;
    SocketScope& operator=(const SocketScope&) = delete;

    [[nodiscard]] bool open() noexcept
    {
        handle_ = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
        if (handle_ == kInvalidSocket) {
            return false;
        }

        sockaddr_in local{};
        local.sin_family = AF_INET;
        local.sin_addr.s_addr = INADDR_ANY;
        local.sin_port = 0;
        return bind(handle_, reinterpret_cast<const sockaddr*>(&local), sizeof(local)) == 0;
    }

    [[nodiscard]] SocketHandle handle() const noexcept { return handle_; }

    [[nodiscard]] std::uint16_t local_port() const noexcept
    {
        sockaddr_in bound{};
#if defined(_WIN32)
        int length = static_cast<int>(sizeof(bound));
#else
        socklen_t length = sizeof(bound);
#endif
        if (getsockname(handle_, reinterpret_cast<sockaddr*>(&bound), &length) != 0) {
            return 0;
        }
        return static_cast<std::uint16_t>(ntohs(bound.sin_port));
    }

private:
    SocketHandle handle_ = kInvalidSocket;
};

#if defined(_WIN32)
class WinsockScope {
public:
    WinsockScope() noexcept
    {
        WSADATA data{};
        started_ = WSAStartup(MAKEWORD(2, 2), &data) == 0;
    }

    ~WinsockScope()
    {
        if (started_) {
            WSACleanup();
        }
    }

    WinsockScope(const WinsockScope&) = delete;
    WinsockScope& operator=(const WinsockScope&) = delete;

    [[nodiscard]] bool started() const noexcept { return started_; }

private:
    bool started_ = false;
};
#endif
}  // namespace

Outcome probe_nat_mapping(Span<const StunServer> servers, std::uint32_t timeout_ms,
                          NetworkProbeResult& out) noexcept
{
    out = NetworkProbeResult{};
    out.probed = true;

    if (servers.size() < 2) {
        out.mapping = NatMapping::Unknown;
        return fail(Status::InvalidArgument,
                    "nat mapping needs two distinct stun servers to be classified");
    }

#if defined(_WIN32)
    WinsockScope winsock;
    if (!winsock.started()) {
        out.mapping = NatMapping::Unknown;
        return fail(Status::Unavailable, "winsock could not be initialised");
    }
#endif

    SocketScope socket_scope;
    if (!socket_scope.open()) {
        out.mapping = NatMapping::Unknown;
        return fail(Status::Unavailable, "could not open a local udp socket");
    }

    out.local_port = socket_scope.local_port();

    StunBinding first{};
    StunBinding second{};
    bool have_first = false;
    bool have_second = false;

    for (std::size_t i = 0; i < servers.size(); ++i) {
        StunBinding binding{};
        if (!query_server(socket_scope.handle(), servers[i], timeout_ms,
                          static_cast<std::uint32_t>(i + 1), binding)) {
            continue;
        }

        ++out.servers_answered;

        if (!have_first) {
            first = binding;
            have_first = true;
            continue;
        }

        second = binding;
        have_second = true;
        break;
    }

    if (!have_first) {
        out.mapping = NatMapping::Blocked;
        return fail(Status::Unavailable, "no stun server answered, udp is likely blocked");
    }

    out.first_public_port = first.port;
    format_address(out.public_address, kAddressTextCapacity, first.address_v4, first.port);

    if (!have_second) {
        out.mapping = NatMapping::Unknown;
        return fail(Status::Unavailable,
                    "only one stun server answered, nat mapping cannot be classified");
    }

    out.second_public_port = second.port;
    out.mapping = first.port == second.port && first.address_v4 == second.address_v4
                      ? NatMapping::EndpointIndependent
                      : NatMapping::AddressDependent;

    return ok();
}
}  // namespace tl::diag
