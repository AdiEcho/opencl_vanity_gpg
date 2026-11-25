$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# External program failures do not throw in Windows PowerShell 5.1.
function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program exited with code $LASTEXITCODE"
    }
}

# Windows exposes Python as python.exe; python3 is not guaranteed to exist.
# Resolve and check it before spending time on the release build.
$python = (Get-Command python.exe -CommandType Application -ErrorAction Stop).Source
Invoke-Checked $python @('--version')

$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$env:PATH = "$cargoBin;$env:PATH"
if (-not (Get-Command rustup.exe -ErrorAction SilentlyContinue)) {
    $installer = Join-Path $env:TEMP 'rustup-init.exe'
    Invoke-WebRequest -UseBasicParsing `
        -Uri 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe' `
        -OutFile $installer
    Invoke-Checked $installer @('-y', '--profile', 'minimal', '--default-toolchain', '1.100.0', '--default-host', 'x86_64-pc-windows-msvc', '--no-modify-path')
}
Invoke-Checked 'rustup' @('toolchain', 'install', '1.100.0-x86_64-pc-windows-msvc', '--profile', 'minimal', '--no-self-update')

$sdkArchive = Join-Path $env:TEMP 'opencl-sdk.zip'
$sdkRoot = Join-Path $env:TEMP 'circleci-opencl-sdk'
Invoke-WebRequest -UseBasicParsing `
    -Uri 'https://github.com/KhronosGroup/OpenCL-SDK/releases/download/v2026.05.29/OpenCL-SDK-v2026.05.29-Win-x64.zip' `
    -OutFile $sdkArchive
if ((Get-FileHash $sdkArchive -Algorithm SHA256).Hash.ToLowerInvariant() -ne '9a85ecef3e9276121ee257122be348bedf29299127b49c0d8286c2d7cf56768d') {
    throw 'OpenCL SDK checksum mismatch'
}
Expand-Archive -Path $sdkArchive -DestinationPath $sdkRoot -Force
$sdk = Join-Path $sdkRoot 'OpenCL-SDK-v2026.05.29-Win-x64'
$libDirectory = (Join-Path $sdk 'lib').Replace('\', '/')
# Use the encoded form so library paths containing spaces remain one argument.
$env:CARGO_ENCODED_RUSTFLAGS = '-L' + [char]0x1f + "native=$libDirectory"
$env:PATH = (Join-Path $sdk 'bin') + ';' + $env:PATH
$env:RUSTUP_TOOLCHAIN = '1.100.0-x86_64-pc-windows-msvc'

Invoke-Checked 'cargo' @('fetch', '--locked')
Invoke-Checked 'cargo' @('build', '--release', '--locked')
Invoke-Checked '.\target\release\opencl_vanity_gpg.exe' @('--help')
Invoke-Checked $python @('.circleci/package-binary.py')
