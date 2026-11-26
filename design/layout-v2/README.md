# Layout v2 do MyBettys (referência)

Protótipo aprovado em 2026-11-26. Original interativo (privado):
https://claude.ai/artifact/GAKSNUwpRSgRM9BAeCCS35

Os `.dc.html` são as telas do protótipo, no formato do canvas de design; os
caminhos `/_blob/...` são os logos de `ui/assets/platforms/`. Servem como
referência visual e não rodam sozinhos. O `app.css` é CSS puro e pode ser
reaproveitado.

## Decisões do usuário
- Menu lateral fixo com a sincronização de cada casa no rodapé.
- **Paleta B · Grafite** (quase preto neutro, destaque violeta `#9785ff`).
  Só tema escuro: sem tema claro.
- Logos das 4 casas (os PNGs de `ui/assets/platforms/`) em toda a interface.
- Sem saudação ("Bem-vindo de volta"); tudo em dólar, no formato `$3,184.70`.
- Fechadas saem de Posições e vão para Histórico (Lista e Calendário).
- Abertas agrupadas por casa.
- Primeira carga como no `AGENTS.md` do MyBettys.
- Não usar o design system ChronoCase.

## Telas
Visão geral, Posições, Histórico, Estatísticas, Conexões, Configurações e a
folha de Estados (carregando, erro, vazio e os seis estados de sync).
