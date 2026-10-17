# Explicit developer/CI opt-in: acquire the exact reviewed compiler, per-user.
# Does not install or run Ulos. Requires Windows PowerShell and HTTPS access.
[CmdletBinding()]
param([string]$Destination = (Join-Path $env:LOCALAPPDATA 'Programs\UlosBuildTools\InnoSetup-6.7.3'))
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $Destination) { throw "Refusing existing compiler destination: $Destination" }
$TempDirectory = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $TempDirectory | Out-Null
try {
    $Installer = Join-Path $TempDirectory 'innosetup-6.7.3.exe'
    Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe' -OutFile $Installer
    # Pin independently recorded in Microsoft's winget 6.7.3 manifest; see packaging/README.md.
    $Expected = '9C73C3BAE7ED48D44112A0F48E66742C00090BDB5BEF71D9D3C056C66E97B732'
    if ((Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash -ne $Expected) { throw 'Inno Setup download SHA256 mismatch' }
    $Signature = Get-AuthenticodeSignature -LiteralPath $Installer
    if ($Signature.Status -ne 'Valid' -or $Signature.SignerCertificate.Subject -notmatch '(^|,\s*)O=Pyrsys B\.V\.(,|$)') {
        throw 'Inno Setup requires a valid Pyrsys B.V. Authenticode signature'
    }
    # No /ALLUSERS, elevation, SDK installation, or system PATH changes.
    $Process = Start-Process -FilePath $Installer -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', "/DIR=`"$Destination`"") -Wait -PassThru
    if ($Process.ExitCode -ne 0) { throw "Inno Setup installation failed: $($Process.ExitCode)" }
    $Compiler = Join-Path $Destination 'ISCC.exe'
    if (-not (Test-Path -LiteralPath $Compiler -PathType Leaf)) { throw 'ISCC.exe missing after installation' }
    Write-Output $Compiler
} finally {
    Remove-Item -LiteralPath $TempDirectory -Recurse -Force
}
