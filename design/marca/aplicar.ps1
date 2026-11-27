# Aplica uma das opções de identidade visual (design/marca/opcao-N) ao app:
# ícones do Windows/instalador (src-tauri/icons) e marca da interface (ui/assets/brand).
# Uso: powershell -File design/marca/aplicar.ps1 -Opcao 3
# Depois, recompile o app para o executável pegar o ícone novo.
param([Parameter(Mandatory = $true)][ValidateSet('2', '3')][string]$Opcao)

$ErrorActionPreference = 'Stop'
$raiz = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$origem = Join-Path $PSScriptRoot "opcao-$Opcao"
$icones = Join-Path $raiz 'src-tauri\icons'
$marca = Join-Path $raiz 'ui\assets\brand'
New-Item -ItemType Directory -Force $marca | Out-Null

$copias = @(
    , @('icone-32.png', "$icones\32x32.png")
    , @('icone-128.png', "$icones\128x128.png")
    , @('icone-256.png', "$icones\128x128@2x.png")
    , @('icone-512.png', "$icones\icon.png")
    , @('icone.ico', "$icones\icon.ico")
    , @('icone.svg', "$marca\icone.svg")
    , @('favicon-16.png', "$marca\favicon-16.png")
    , @('favicon-32.png', "$marca\favicon-32.png")
)
foreach ($par in $copias) {
    Copy-Item -LiteralPath (Join-Path $origem $par[0]) -Destination $par[1] -Force
}

# Logo = símbolo (SVG) + palavra "Betty" em texto real. Desenhada como texto, a palavra
# ganha ClearType igual ao resto da interface; como contorno vetorial ela ficava borrada.
# A palavra usa betty-wordmark.woff2 (Inter Display ExtraBold, a fonte do logo do designer).
#
# marca.svg: só o símbolo do logo-escuro.svg, com o viewBox recortado ao desenho
# (caixa medida no WebView), para as bordas retas caírem em pixel inteiro.
$utf8 = New-Object Text.UTF8Encoding $false
$caixa = @{ '2' = @(22, 20, 122, 128); '3' = @(24, 14.28, 118, 124.42) }[$Opcao]  # x0 y0 x1 y1
$inv = [Globalization.CultureInfo]::InvariantCulture
$n = { param($v) ([double]$v).ToString('0.####', $inv) }
$larg = $caixa[2] - $caixa[0]; $alt = $caixa[3] - $caixa[1]

$svg = [IO.File]::ReadAllText((Join-Path $origem 'logo-escuro.svg'))
$cabecalho = 'width="428" height="144" viewBox="0 0 428 144"'
$inicioTexto = $svg.IndexOf('<g fill="#f4f4f6"')
if (-not $svg.Contains($cabecalho) -or $inicioTexto -lt 0) { throw "logo-escuro.svg da opção $Opcao mudou: revise o recorte." }
$svg = $svg.Substring(0, $inicioTexto) + '</svg>'
$svg = $svg.Replace($cabecalho, "width=""$(& $n $larg)"" height=""$(& $n $alt)"" viewBox=""$(& $n $caixa[0]) $(& $n $caixa[1]) $(& $n $larg) $(& $n $alt)""")
[IO.File]::WriteAllText("$marca\marca.svg", $svg, $utf8)

# Posição da palavra em relação ao símbolo, nas unidades do logo original.
# No SVG do designer a palavra é translate(150 22) scale(s) translate(-96 -1490):
# origem da escrita em x = 150 - 96s, topo das maiúsculas em y = 22, 1 em = 2048s.
$s = 0.05241090146750524
$css = @"
/* Gerado por design/marca/aplicar.ps1 (opção $Opcao). Não edite à mão. */
:root {
  --marca-alt-un: $(& $n $alt);
  --palavra-dx-un: $(& $n (150 - 96 * $s - $caixa[2]));
  --palavra-dy-un: $(& $n (22 - $caixa[1]));
  --palavra-em-un: $(& $n (2048 * $s));
}
"@
[IO.File]::WriteAllText("$marca\marca.css", $css, $utf8)
Remove-Item -LiteralPath "$marca\logo.svg" -ErrorAction SilentlyContinue
Write-Host "Opção $Opcao aplicada. Recompile o app para atualizar o ícone do executável."
