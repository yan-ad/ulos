# Explicit per-user registration only; never sets .torrent defaults or UserChoice.
# Run deliberately on Windows after installing the executable in a stable location.
[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Medium')]
param(
    [Parameter(Mandatory = $true)]
    [string]$ExecutablePath
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Windows is required.' }
$exe = Get-Item -LiteralPath $ExecutablePath -ErrorAction Stop
if ($exe.PSIsContainer -or $exe.Extension -ine '.exe') { throw 'Select a local executable file.' }
if ($exe.FullName.StartsWith('\\') -or $exe.FullName.Contains('"')) { throw 'Select a local executable path.' }
$command = '"' + $exe.FullName + '" "%1"'
if ($PSCmdlet.ShouldProcess('HKCU Ulos .torrent Open With registration', 'Register without changing default')) {
    $classes = 'HKCU:\Software\Classes'
    $progId = "$classes\Ulos.Torrent"
    New-Item -Path "$progId\shell\open\command" -Force | Out-Null
    Set-Item -LiteralPath $progId -Value 'BitTorrent metainfo (Ulos)'
    Set-Item -LiteralPath "$progId\shell\open\command" -Value $command
    New-ItemProperty -LiteralPath $progId -Name FriendlyTypeName -Value 'BitTorrent metainfo (Ulos)' -PropertyType String -Force | Out-Null
    $openWith = "$classes\.torrent\OpenWithProgids"
    New-Item -Path $openWith -Force | Out-Null
    New-ItemProperty -LiteralPath $openWith -Name 'Ulos.Torrent' -Value '' -PropertyType String -Force | Out-Null
    $caps = 'HKCU:\Software\Ulos\Capabilities'
    New-Item -Path "$caps\FileAssociations" -Force | Out-Null
    New-ItemProperty -LiteralPath $caps -Name ApplicationName -Value 'Ulos' -PropertyType String -Force | Out-Null
    New-ItemProperty -LiteralPath $caps -Name ApplicationDescription -Value 'Download manager supporting local torrent metainfo' -PropertyType String -Force | Out-Null
    New-ItemProperty -LiteralPath "$caps\FileAssociations" -Name '.torrent' -Value 'Ulos.Torrent' -PropertyType String -Force | Out-Null
    New-Item -Path 'HKCU:\Software\RegisteredApplications' -Force | Out-Null
    New-ItemProperty -LiteralPath 'HKCU:\Software\RegisteredApplications' -Name Ulos -Value 'Software\Ulos\Capabilities' -PropertyType String -Force | Out-Null
    Write-Output 'Registered Ulos for this user. Choose Open With explicitly; default association was not changed.'
}
