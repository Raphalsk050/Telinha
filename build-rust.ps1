<#
Compila o Telinha em Rust: o motor de captura (telinha.exe, em C++) e o app, que leva o motor
dentro. O resultado e um arquivo so.

  .\build-rust.ps1          motor + client\dist\Telinha.exe
  .\build-rust.ps1 -Run     motor + abre o app de desenvolvimento, sem empacotar
  .\build-rust.ps1 -Test    roda os testes do lado Rust

O app empacotado aqui se atualiza sozinho como o da esteira de release (TELINHA_UPDATES=1): ele
troca de lugar com uma versao publicada mais nova que a do version.json. O -Run nao se atualiza.
#>
param(
    [switch]$Run,
    [switch]$Test
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$root = $PSScriptRoot
$client = Join-Path $root 'client'
$dist = Join-Path $client 'dist'

function Invoke-Step([string]$Title, [scriptblock]$Action) {
    Write-Host "`n==> $Title" -ForegroundColor Cyan
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $global:LASTEXITCODE = 0
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw "$Title falhou (codigo $LASTEXITCODE)"
    }
    Write-Host ('    ok em {0:n0}s' -f $watch.Elapsed.TotalSeconds) -ForegroundColor Green
}

Invoke-Step 'compilando o motor de captura' { & (Join-Path $root 'build.ps1') -ExeOnly }

Push-Location $client
try {
    if ($Test) {
        # Os testes de voz abrem conexoes de verdade e fazem o Windows perguntar sobre o firewall,
        # entao rodam so a pedido: cargo test -p telinha-voice
        Invoke-Step 'testando o lado Rust' { cargo test -p telinha-core -p telinha }
        return
    }
    if ($Run) {
        Write-Host "`n==> abrindo o app, feche a janela para voltar ao terminal" -ForegroundColor Cyan
        cargo run -p telinha
        return
    }
    $env:TELINHA_UPDATES = '1'
    try {
        Invoke-Step 'compilando o app' { cargo build --release -p telinha }
    }
    finally {
        Remove-Item Env:TELINHA_UPDATES -ErrorAction SilentlyContinue
    }
    New-Item -ItemType Directory -Force $dist | Out-Null
    Copy-Item (Join-Path $client 'target\release\telinha.exe') (Join-Path $dist 'Telinha.exe') -Force
    $size = (Get-Item (Join-Path $dist 'Telinha.exe')).Length / 1MB
    Write-Host ("`nPronto: {0} ({1:n1} MB)" -f (Join-Path $dist 'Telinha.exe'), $size) -ForegroundColor Green
}
finally {
    Pop-Location
}
