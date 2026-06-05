# kern one-line installer (Windows).
#
#   irm https://raw.githubusercontent.com/yesitsfebreeze/relay-kern/master/install.ps1 | iex
#
# Downloads the prebuilt kern.exe for this platform from the latest GitHub
# release and installs it to %LOCALAPPDATA%\kern\bin (override with
# $env:KERN_BIN_DIR), adding that directory to the user PATH.
$ErrorActionPreference = 'Stop'

$repo = 'yesitsfebreeze/relay-kern'
$binDir = if ($env:KERN_BIN_DIR) { $env:KERN_BIN_DIR } else { "$env:LOCALAPPDATA\kern\bin" }

$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -ne 'AMD64') {
  Write-Error "kern: no prebuilt Windows binary for $arch (build from source)"
}
$target = 'x86_64-pc-windows-msvc'
$url = "https://github.com/$repo/releases/latest/download/kern-$target.zip"

Write-Host "kern: downloading $target ..."
New-Item -ItemType Directory -Force -Path $binDir | Out-Null
$zip = Join-Path $env:TEMP "kern-$target.zip"
try {
  Invoke-WebRequest -Uri $url -OutFile $zip
} catch {
  Write-Error "kern: download failed ($url). No release yet? See https://github.com/$repo/releases"
}
Expand-Archive -Path $zip -DestinationPath $binDir -Force
Remove-Item $zip -ErrorAction SilentlyContinue

Write-Host "kern: installed to $binDir\kern.exe"

# Add to user PATH if missing.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $binDir) {
  [Environment]::SetEnvironmentVariable('Path', "$userPath;$binDir", 'User')
  Write-Host "kern: added $binDir to your user PATH (restart the shell to pick it up)"
}
Write-Host "kern: next - register the MCP server:  claude mcp add kern -- `"$binDir\kern.exe`" mcp"
