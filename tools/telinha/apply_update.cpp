#include "apply_update.hpp"

#include <windows.h>

#include <shellapi.h>

#include <cstdio>
#include <cwchar>

namespace {

constexpr DWORD kAppExitWaitMs = 60000;
constexpr DWORD kRetryMs = 500;
constexpr int kCopyAttempts = 40;
// O executavel ainda fechando tambem pode responder "acesso negado" por um instante.
constexpr int kDeniedAttempts = 6;
constexpr DWORD kPathCapacity = 1024;

struct Text {
    wchar_t value[kPathCapacity] = L"";

    [[nodiscard]] bool read(const wchar_t* name) noexcept
    {
        const DWORD length = GetEnvironmentVariableW(name, value, kPathCapacity);
        if (length == 0 || length >= kPathCapacity) {
            value[0] = L'\0';
            return false;
        }
        return true;
    }

    [[nodiscard]] bool is(const wchar_t* other) const noexcept
    {
        return std::wcscmp(value, other) == 0;
    }
};

void wait_for_app(const Text& process_id) noexcept
{
    const DWORD id = std::wcstoul(process_id.value, nullptr, 10);
    if (id == 0) {
        return;
    }
    const HANDLE process = OpenProcess(SYNCHRONIZE, FALSE, id);
    if (process != nullptr) {
        WaitForSingleObject(process, kAppExitWaitMs);
        CloseHandle(process);
    }
}

void write_mark(const Text& file, const Text& version) noexcept
{
    if (file.value[0] == L'\0') {
        return;
    }
    FILE* out = nullptr;
    if (_wfopen_s(&out, file.value, L"w") == 0 && out != nullptr) {
        std::fwprintf(out, L"%ls\n", version.value);
        std::fclose(out);
    }
}

// Uma pasta protegida, como a raiz do C:, so aceita a copia com a permissao do Windows. O pedido
// aparece em nome do prompt de comando, que e quem faz a copia.
[[nodiscard]] bool copy_with_permission(const Text& source, const Text& target) noexcept
{
    Text shell;
    wchar_t command[kPathCapacity * 2 + 32] = L"";
    if (!shell.read(L"ComSpec") || std::wcschr(source.value, L'"') != nullptr ||
        std::wcschr(target.value, L'"') != nullptr ||
        swprintf_s(command, L"/c copy /y \"%ls\" \"%ls\"", source.value, target.value) < 0) {
        return false;
    }

    SHELLEXECUTEINFOW request = {};
    request.cbSize = sizeof(request);
    request.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    request.lpVerb = L"runas";
    request.lpFile = shell.value;
    request.lpParameters = command;
    request.nShow = SW_HIDE;
    if (ShellExecuteExW(&request) == 0 || request.hProcess == nullptr) {
        return false;
    }

    WaitForSingleObject(request.hProcess, INFINITE);
    DWORD exit_code = 1;
    GetExitCodeProcess(request.hProcess, &exit_code);
    CloseHandle(request.hProcess);
    return exit_code == 0;
}

}  // namespace

int apply_update() noexcept
{
    Text source;
    Text target;
    Text version;
    Text process_id;
    Text applied;
    Text failed;
    Text relaunch;
    Text elevate;
    if (!source.read(L"TELINHA_UPDATE_SOURCE") || !target.read(L"TELINHA_UPDATE_TARGET")) {
        return 2;
    }
    (void)version.read(L"TELINHA_UPDATE_VERSION");
    (void)process_id.read(L"TELINHA_UPDATE_PID");
    (void)applied.read(L"TELINHA_UPDATE_APPLIED");
    (void)failed.read(L"TELINHA_UPDATE_FAILED");
    (void)relaunch.read(L"TELINHA_UPDATE_RELAUNCH");
    (void)elevate.read(L"TELINHA_UPDATE_ELEVATE");

    // O Windows so deixa escrever por cima do executavel depois que o app fecha. Copiar por cima,
    // em vez de trocar de nome, mantem o arquivo no mesmo lugar e com as mesmas permissoes.
    wait_for_app(process_id);

    bool done = false;
    int denied = 0;
    for (int attempt = 0; attempt < kCopyAttempts && !done && denied < kDeniedAttempts; ++attempt) {
        if (CopyFileW(source.value, target.value, FALSE) != 0) {
            done = true;
        } else {
            denied = GetLastError() == ERROR_ACCESS_DENIED ? denied + 1 : 0;
            Sleep(kRetryMs);
        }
    }
    const bool may_ask = elevate.is(L"1");
    if (!done && denied >= kDeniedAttempts && may_ask) {
        done = copy_with_permission(source, target);
    }

    if (done) {
        write_mark(applied, version);
        DeleteFileW(source.value);
    } else if (may_ask) {
        write_mark(failed, version);
    }
    if (relaunch.is(L"1")) {
        ShellExecuteW(nullptr, L"open", target.value, nullptr, nullptr, SW_SHOWNORMAL);
    }
    return done ? 0 : 1;
}
