# Instalador por comando de Claude Bar (Windows).
#
#   irm https://raw.githubusercontent.com/daybigo/ClaudeBar/main/install.ps1 | iex
#
# Descarga el instalador de la ultima release y lo ejecuta. Se instala en tu
# usuario, sin permisos de administrador.

$ErrorActionPreference = 'Stop'
$repo = 'daybigo/ClaudeBar'

Write-Host "Buscando la ultima version de Claude Bar..."
$headers = @{ 'User-Agent' = 'claudebar-install'; 'Accept' = 'application/vnd.github+json' }
$rel = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers $headers

$asset = $rel.assets | Where-Object { $_.name -like '*-setup.exe' } | Select-Object -First 1
if (-not $asset) {
    throw "No se encontro un instalador (*-setup.exe) en la release $($rel.tag_name)."
}

$out = Join-Path $env:TEMP $asset.name
$mb = [math]::Round($asset.size / 1MB, 1)
Write-Host "Descargando $($asset.name) ($mb MB) [$($rel.tag_name)]..."
Invoke-WebRequest $asset.browser_download_url -OutFile $out -UseBasicParsing -Headers $headers

Write-Host "Ejecutando el instalador..."
Start-Process -FilePath $out -Wait

Write-Host ""
Write-Host "Listo. Claude Bar deberia aparecer en la bandeja del sistema."
Write-Host "Necesita Claude Code instalado e iniciado sesion para mostrar tu uso."
