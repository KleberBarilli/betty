# Memória do Betty aberto: o processo Rust e cada processo do WebView2 dele.
#
#   powershell -File medir_memoria.ps1 [nome do exe, padrão betty.exe]
#
# Memória privada (PrivateMemorySize64, a "Bytes particulares" do Windows) e o
# pico do betty.exe (PeakPagedMemorySize64). Os processos do WebView2 são os
# msedgewebview2.exe com --webview-exe-name=<exe>.
param([string]$Exe = 'betty.exe')

$mb = { param($b) [math]::Round($b / 1MB) }
$principal = Get-Process -Name ($Exe -replace '\.exe$', '') -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $principal) { Write-Output "$Exe não está aberto"; exit 1 }
$linhas = @([pscustomobject]@{ Processo = $Exe; Papel = 'Rust'; PrivadaMB = & $mb $principal.PrivateMemorySize64; PicoMB = & $mb $principal.PeakPagedMemorySize64 })
$webview = Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -like "*--webview-exe-name=$Exe*" }
foreach ($p in $webview) {
    $papel = 'browser'
    if ($p.CommandLine -match '--type=(\S+)') { $papel = $Matches[1] }
    if ($papel -eq 'utility' -and $p.CommandLine -match '--utility-sub-type=(\S+)') { $papel = ($Matches[1] -split '\.')[0] }
    if ($papel -eq 'renderer') { $papel = 'página' }
    $proc = Get-Process -Id $p.ProcessId -ErrorAction SilentlyContinue
    if ($proc) { $linhas += [pscustomobject]@{ Processo = 'msedgewebview2'; Papel = $papel; PrivadaMB = & $mb $proc.PrivateMemorySize64; PicoMB = '' } }
}
$linhas | Format-Table -AutoSize | Out-String -Width 200
$total = ($linhas | Measure-Object -Property PrivadaMB -Sum).Sum
Write-Output "total privado: $total MB"
