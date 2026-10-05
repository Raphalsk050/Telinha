#pragma once

// Troca o executavel do app pela versao nova ja baixada, depois que o app fecha. O pedido vem do
// app em variaveis TELINHA_UPDATE_*, montadas em desktop/src/updater.js.
[[nodiscard]] int apply_update() noexcept;
