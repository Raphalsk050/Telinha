<#
Compila o telinha.exe e gera o app do Telinha usando todos os nucleos da maquina.

  .\build.ps1            telinha.exe + desktop\dist\Telinha-<versao>-portable.exe
  .\build.ps1 -Run       telinha.exe + abre o app direto, sem empacotar (o jeito mais rapido de testar)
  .\build.ps1 -ExeOnly   so o telinha.exe
#>
param(
    [switch]$Run,
    [switch]$ExeOnly
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$root = $PSScriptRoot
$buildDir = Join-Path $root 'build\msvc-release'
$exe = Join-Path $buildDir 'tools\telinha\Release\telinha.exe'
$desktop = Join-Path $root 'desktop'

function Format-Elapsed([Diagnostics.Stopwatch]$Watch) {
    $elapsed = $Watch.Elapsed
    if ($elapsed.TotalMinutes -lt 1) {
        return '{0}s' -f $elapsed.Seconds
    }
    '{0}min{1:00}s' -f [math]::Floor($elapsed.TotalMinutes), $elapsed.Seconds
}

function Invoke-Step([string]$Title, [scriptblock]$Action) {
    Write-Host "`n==> $Title" -ForegroundColor Cyan
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $global:LASTEXITCODE = 0
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw "$Title falhou (codigo $LASTEXITCODE)"
    }
    Write-Host "    ok em $(Format-Elapsed $watch)" -ForegroundColor Green
}

function Close-RunningTelinha {
    $running = @(Get-CimInstance Win32_Process | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith("$root\", [StringComparison]::OrdinalIgnoreCase)
    })
    if ($running.Count -eq 0) {
        return
    }
    $names = ($running | ForEach-Object { "$($_.Name) ($($_.ProcessId))" }) -join ', '
    Write-Host "Aberto a partir do projeto, trava os arquivos do build: $names" -ForegroundColor Yellow
    if ((Read-Host 'Fechar agora? [S/n]') -match '^[nN]') {
        throw 'Feche o Telinha e rode o build de novo.'
    }
    Stop-Process -Id $running.ProcessId -Force -ErrorAction SilentlyContinue
    Wait-Process -Id $running.ProcessId -Timeout 10 -ErrorAction SilentlyContinue
}

function Get-NewestMsvcToolset {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $vs = & $vswhere -latest -products '*' -property installationPath
    $newest = Get-ChildItem (Join-Path $vs 'VC\Tools\MSVC') -Directory |
        Sort-Object { [version]$_.Name } |
        Select-Object -Last 1
    $version = [version]$newest.Name
    '{0}.{1}' -f $version.Major, $version.Minor
}

$total = [Diagnostics.Stopwatch]::StartNew()
$previousCl = $env:CL
Push-Location $root
try {
    Close-RunningTelinha

    if (-not (Test-Path (Join-Path $buildDir 'CMakeCache.txt'))) {
        # The pinned libwebrtc needs the 14.44 STL, and the v143 default on this machine is still 14.38.
        $toolset = Get-NewestMsvcToolset
        Invoke-Step "configurando o CMake com o MSVC $toolset" {
            cmake --preset msvc-release -T "v143,version=$toolset"
        }
    }

    # cl.exe reads extra flags from CL. /MP compiles the files of each project in parallel, and
    # unlike UseMultiToolTask it still rebuilds .obj files that were deleted.
    $env:CL = "/MP $previousCl"
    Invoke-Step 'compilando o telinha.exe' {
        cmake --build $buildDir --config Release --target telinha --parallel
    }

    if (-not $ExeOnly) {
        if (-not (Test-Path (Join-Path $desktop 'node_modules'))) {
            Invoke-Step 'instalando as dependencias do app' { npm --prefix $desktop ci }
        }
        if ($Run) {
            Write-Host "`n==> abrindo o app, feche a janela para voltar ao terminal" -ForegroundColor Cyan
            npm --prefix $desktop start
            return
        }
        Invoke-Step 'gerando o Telinha portatil' {
            Write-Host '    leva ~2min e o log fica parado em "building target=portable" enquanto comprime' -ForegroundColor DarkGray
            npm --prefix $desktop run dist
        }
    }

    Write-Host "`nPronto em $(Format-Elapsed $total)" -ForegroundColor Green
    Write-Host "  telinha.exe: $exe"
    if (-not $ExeOnly) {
        $portable = Get-ChildItem (Join-Path $desktop 'dist\*.exe') | Sort-Object LastWriteTime | Select-Object -Last 1
        Write-Host "  app:         $($portable.FullName)"
    }
}
finally {
    $env:CL = $previousCl
    Pop-Location
}
