# Desempenho do Betty: o que foi medido, o que foi feito e o que falta

Este documento é o plano de desempenho do Betty (Rust + Tauri 2 + WebView2). Ele
existe para que qualquer dev consiga continuar o trabalho sem o histórico da
conversa em que ele começou.

**Regra que vale para tudo aqui:** o comportamento tem que continuar idêntico.
Títulos, categorias, datas, PnL, índice e ledger não podem mudar por causa de uma
otimização. Toda mudança que mexe em dados é validada contra o código atual com
um oráculo (seção [Como validar](#como-validar)). O código original do Electron
(repositório `smart/betty`, com o `AGENTS.md` das regras de negócio) é a
referência de comportamento.

## Sumário

- [Números de partida](#números-de-partida)
- [O que já foi feito](#o-que-já-foi-feito)
- [Como validar](#como-validar)
- [O que falta](#o-que-falta)
- [O que não vale a pena mexer](#o-que-não-vale-a-pena-mexer)
- [Armadilhas conhecidas](#armadilhas-conhecidas)

## Números de partida

Medidos em 2026-11-27 numa conta real: 4.232 apostas, sendo 2.281 da Polymarket,
1.944 da Kalshi e 7 de Predict.fun/SX Bet.

| Medida | Antes | Depois do que já foi feito |
|---|---|---|
| Banco (`betty.db`) | 284 MB | 151 MB |
| Ledger da Polymarket | 163 MB | 27 MB |
| `/api/history/closed/metadata` (6 lotes da abertura) | 153 MB, 1,1 s, pico de ~800 MB | ~22 MB, 148 ms, pico de 151 MB |
| Ledger inteiro (`GET /api/history/closed`, reindexação) | 158 MB, pico de ~1,5 GB | 29 MB, pico de 347 MB |
| `/api/polymarket-metadata/cache` (lote grande da abertura) | 32,8 MB | 3,7 MB |
| Interface visível ao abrir | só depois da sincronização inteira | dados salvos logo; confirmados no fim |
| Rotas normais do histórico (query, stats, pnl, calendário) | 3–18 ms | igual |
| `/api/me` durante a manutenção da abertura (compactação + `VACUUM`, 4,4 s) | 4,4–5,1 s, esperando a manutenção inteira | mediana 0,2 ms, máximo 1,1 ms |

### No app instalado

Medido em 2026-11-27 com `medir_memoria.ps1` e `medir_app.mjs`. O build novo tem tudo até a seção 6 de [O que já foi feito](#o-que-já-foi-feito). A memória foi lida 1 minuto depois de abrir, com o app parado.

| Processo (memória privada) | Build antigo | Build novo |
|---|---|---|
| `betty.exe` (Rust) | 212–215 MB, com pico de 1.174 MB | 233–278 MB logo depois de abrir, pico de 404–559 MB (ver item 2 de [O que falta](#o-que-falta)) |
| WebView2: página | 193–225 MB | 77–88 MB |
| WebView2: GPU | 83–131 MB | 100–103 MB |
| WebView2: browser, rede, storage | 37 + 11 + 11 MB | 36 + 12 + 8 MB |
| Banco (`betty.db`) | 284 MB | 134 MB |

| Abertura (desde a navegação da página) | Build antigo | Build novo |
|---|---|---|
| Dados salvos na tela | só depois da sincronização inteira; 3,9 s num build de teste | 0,39–0,45 s |
| Números confirmados | 11,5 s num build de teste | 4,8–5,4 s |
| Primeira abertura depois de atualizar | — | confirmação em 21,5 s: compactação única, índice, `VACUUM` e o reenvio único das apostas da Polymarket ao ledger |

A sincronização é dominada pela rede: activity da Polymarket 4,5 s, fechadas da Polymarket 3,4 s, liquidações da Kalshi 2,3 s, sync da Predict 1,4 s.

## O que já foi feito

### 1. Metadados compactados no ledger (`66326c3`)

**Problema:** cada aposta da Polymarket guardava em `_sportMeta` o evento inteiro do Gamma, com até 508 mercados e suas descrições. Isso dava 70 KB por aposta, 97% do ledger.

**Solução:** `ledger::compactar_meta_esporte` reduz `_sportMeta` assim:
- mantém só o mercado exato da aposta, inteiro;
- mantém `texts` intacto;
- grava `eventTexts` já calculado com a lista inteira, como `sinaisDaAposta()` faria.

Só compacta quando duas condições valem juntas:
- o `exactMarket()` e o `selectedMarket` de `normalizarNomeMercadoPolymarket()` apontam para o mesmo mercado;
- a forma que a tabela recebe (`history::meta_da_tabela`) não muda.

**Onde roda:**
- Na gravação: `ledger::gravar`.
- Numa migração única: `ledger::compactar_ledger_uma_vez`, marcada como `compact_polymarket_sport_meta_v1` em `database_maintenance`.
- Depois da migração, um `VACUUM` (`db::recuperar_espaco`) roda quando sobram 32 MB ou mais livres. Tudo isso fica em segundo plano, em `lib.rs::manutencao_do_banco`.

**No mesmo commit:**
- As versões do índice (categoria, título e data) saíram do localStorage e foram para a tabela `history_index_versions`. `GET .../index/status` devolve `versions` e `POST /api/history/closed/index/versions` grava.
- O cache das fechadas (`betty:polyClosedEnriched:`) ficou só na memória, porque passava da cota.
- Os caches em memória do Rust que só valem dentro do prazo saem a cada minuto. O proxy do Gamma ganhou um teto de 16 MB.

### 2. Abertura com dados salvos (`9ceceb3`)

**Problema:** a interface ficava escondida (`body.initial-loading`) até a primeira sincronização inteira terminar.

**Solução:** `abrirComDadosSalvos()` mostra logo o índice do SQLite, as posições do cache local e os saldos salvos (`betty:saldosSalvos:v1`), com o selo `#seloDadosSalvos` ("Atualizando") no topo.

**Consistência:**
- Antes de a sincronização gravar qualquer coisa, a interface chama `POST /api/history/snapshot`.
- Isso abre uma conexão só de leitura com uma transação aberta (`api::Instantaneo`). Em WAL, ela enxerga o banco como estava naquele momento.
- As leituras com `snapshot=1` (stats, pnl, calendar, query, layout) saem dela, então nunca veem o índice pela metade.
- Ela expira em 15 minutos.
- No JS, os predicados `versaoLeituraHistorico()`, `indiceHistoricoLegivel()` e `urlLeituraHistorico()` decidem de onde cada leitura sai.

**Saída dos dados salvos:** `encerrarDadosSalvos()` só roda quando o snapshot vivo e o índice foram confirmados e a Polymarket e a Kalshi responderam. Se não, o selo vira um aviso amarelo e os dados salvos continuam na tela.

**No mesmo commit:** as fechadas da Polymarket passaram a ser pedidas de 500 em 500 em vez de 50. O backend já tinha a lista inteira em memória e só fatiava.

### 3. Cache de metadados por evento recortado (`fef62c7`)

**Problema:** a cada abertura, `buscarDatasInicioPorSlug()` pedia ao SQLite o evento inteiro de cerca de 300 eventos (as posições, incluindo perdas não resgatadas), 36 MB numa chamada.

**Solução:**
- O pedido leva `need` por evento (`assets`, `conditions`, `slugs` das apostas).
- `poly::recortar_evento` devolve:
  - **datas:** o evento inteiro e só os mercados das apostas;
  - **esporte:** `texts`, os tipos de mercado, `eventTexts` calculado e os mercados das apostas, mais um mercado com `tokenIds`, porque a interface exige um.
- Apostas do ledger sem `_confirmedClosedTs` também entram no recorte.
- Os caches `betty_sport_meta_v3` e `betty_gamma_date_meta_v1` saíram do localStorage. Eles nunca chegavam a ser gravados (passavam da cota), então a leitura já voltava sempre vazia.

### 4. Leituras do SQLite em paralelo (`90c71ab`)

**Problema:** `db::Db` era uma conexão só, atrás de um `Mutex`. Toda leitura da tela esperava qualquer gravação: ledger, índice, a manutenção da abertura e os caches da Kalshi e da Polymarket. Na primeira abertura depois da atualização, `/api/me` esperou a compactação inteira.

**Solução:**
- `db::Db` tem uma conexão de escrita (`call`) e duas só de leitura (`ler`, `db::CONEXOES_DE_LEITURA`), abertas por `db::abrir_leitura` (`SQLITE_OPEN_READ_ONLY`, 8 MB de cache cada, contra 32 MB na de escrita). Em WAL elas não esperam a de escrita e veem tudo o que já foi confirmado.
- `ler` pega a primeira conexão livre, num rodízio. Com as duas ocupadas, espera a da vez. Uma gravação dentro de `ler` falha com `SQLITE_READONLY`.
- **Vão para `ler`:**
  - `/api/me`, `GET /api/settings` e as conexões devolvidas depois de conectar uma casa;
  - todas as rotas `GET` do histórico, `lookup`, `metadata` e a leitura do `/api/polymarket-metadata/cache` (a gravação, com `entries`, continua em `call`);
  - as leituras de cache e de credenciais em `kalshi.rs`, `poly.rs`, `poly_datas.rs` e `tracked.rs`.
- **Tudo o que grava continua em `call`**, uma gravação por vez, como antes.
- **Métricas pendentes:** `history::garantir_metricas` não grava numa conexão de leitura. As rotas que a chamavam (query, stats, calendar, pnl) passam antes por `api::completar_metricas`: ela confere numa conexão de leitura (`history::metricas_pendentes`) e, só se faltar alguma, completa na de escrita. Só bancos de versões antigas têm métricas pendentes, porque `ledger::salvar_indice` sempre grava com elas. O `index/status` continua sem completar, como antes.
- **`mmap`:** as conexões de leitura abrem sem `mmap_size` e só o ligam no fim de `lib.rs::manutencao_do_banco` (`Db::mapear_leitura`). No Windows, um arquivo mapeado por outra conexão não pode ser truncado, e o `VACUUM` deixaria de devolver o espaço sem dar erro (ver [Armadilhas conhecidas](#armadilhas-conhecidas)). Com `mmap`, as varreduras grandes (ledger inteiro, exportação, metadados) ficam 1,3 a 1,6 vez mais rápidas do que sem.
- O estado salvo (`api::Instantaneo`) guarda uma `db::Conexao`, a mesma conexão com mutex que a de escrita usa.

**Validação:**
- 63 rotas de leitura numa cópia do banco real, antes e depois (`rotas_leitura.py` e `comparar_rotas.py`): 62 idênticas byte a byte. A outra é um controle de propósito: o PnL sem `now` fixo, que só difere no instante do último ponto da série.
- Tempos dessas rotas iguais aos de antes, dentro do ruído (ledger inteiro 789 → 612 ms, exportação 434 → 479 ms, metadados 222 → 195 ms).
- Testes:
  - `db::tests::leitura_nao_espera_a_gravacao`: lê com a escrita presa no meio de uma transação, as duas conexões de leitura ao mesmo tempo, e confere que a gravação aparece depois do commit;
  - `api::tests::rotas_de_leitura_nao_esperam_a_gravacao`: 8 rotas respondem com a escrita presa;
  - `api::tests::metricas_pendentes_sao_completadas_antes_da_leitura`;
  - `db::tests::vacuum_encolhe_o_arquivo_com_as_leituras_abertas`.
- Controle negativo: com `ler` mandado para a conexão de escrita, os dois testes de espera falham.
- Manutenção real (`api::tests::medir_leitura_durante_a_manutencao`, numa cópia sem compactar): o banco foi de 284 MB para 151 MB com as conexões de leitura abertas, e `/api/me` respondeu 44 vezes durante os 4,4 s, com mediana de 0,2 ms. Com `BETTY_CONEXOES_DE_LEITURA=0` (uma conexão só, como antes), o único pedido esperou 4,4 s.

**Corrigido depois:** a conexão de escrita fechava antes das de leitura. Uma conexão só de leitura não consegue juntar o WAL ao arquivo, então o `betty.db-wal` ficava com as últimas gravações (23 MB depois da manutenção) e uma cópia só do `betty.db` saía sem elas. Agora os campos de `db::Db` estão na ordem em que precisam fechar (`db::tests::fechar_junta_o_wal_ao_arquivo`).

### 5. Cache de metadados mais leve

**Polymarket (`POST /api/polymarket-metadata/cache`):** 260 ms por lote grande da abertura (400 eventos, 10 MB lidos), em três partes:

- **Necessidades, 65–90 ms:** `poly::necessidades` rodava `json_extract` em todas as apostas da Polymarket para achar as sem `_confirmedClosedTs` (hoje nenhuma).
  - Agora há um índice parcial com essa condição (`db::INDICE_SEM_CONFIRMACAO`), criado pela manutenção da abertura (`db::criar_indices_da_manutencao`), e a consulta usa `INDEXED BY` quando ele existe. Caiu para 2–5 ms.
  - **Tem de ser parcial.** Um índice comum em `closed_bets_ledger(user_id, platform, ...)` fez o SQLite ler o ledger por ele, e as 539 apostas com o mesmo `first_seen_at` de outra saíram em outra ordem no `GET /api/history/closed`. O índice parcial só entra numa consulta que repete a condição dele (`history::tests::indices_da_manutencao_nao_mudam_os_planos_do_ledger`, que falha com o índice comum).
- **Decodificar e recortar, 110–160 ms:** cada evento virava um `Value` inteiro antes do recorte.
  - Agora `poly::entrada_rapida` lê o evento com `json_bruto` (só os níveis que importam, com o texto original de cada valor), valida e recorta como `metadado_valido` + `recortar_evento`, e monta a resposta em texto. Caiu para 65–95 ms.
  - Qualquer forma fora do comum (escape `\u` ou `\/`, chave repetida, mercado que não é objeto...) cai no caminho antigo (`poly::entrada_completa`), que continua sendo a referência.
- **Validação:**
  - `poly::tests::oraculo_caminho_rapido`, na cópia do banco real: 868 eventos e 4.310 casos (sem pedido, todas as apostas do evento, só a primeira, pedido vazio, um terço dos mercados), todos pelo caminho rápido e com os mesmos bytes do caminho com `Value`;
  - `poly::tests::caminho_rapido_igual_ao_completo` cobre as variantes raras.

**Kalshi (`kalshi_metadata_cache`):** 41,8 MB, sendo 36,6 MB de eventos. Em cada evento, `markets` é cópia de `_markets` (11,3 MB no total) e `_milestone` é cópia de um item de `_milestones` (6,4 MB).

- `kalshi::compactar_evento` troca as duas cópias por marcadores na mesma posição, e `kalshi::expandir_evento` desfaz na leitura (`meta_varios`). Quem lê recebe o mesmo `Value` de antes.
- A comparação é pelo texto: o `==` do `serde_json` ignora a ordem das chaves.
- `history::metadados_kalshi` lê o cache direto, mas só `title`, `_teamsTitle` e `product_metadata`, que não mudam.
- Os eventos já gravados são compactados uma vez na manutenção (`kalshi::compactar_cache_uma_vez`, marcador `compact_kalshi_event_cache_v1`), antes do `VACUUM`.
- **Validação:** `kalshi::tests::oraculo_cache_de_eventos` diz que os 1.748 eventos da cópia voltam com o mesmo texto, e o cache de eventos vai de 36,6 MB para 18,9 MB.

**Rotas:** 64 rotas de leitura, código antigo contra o novo depois da manutenção nova (`rotas_leitura.py` ganhou o pedido da abertura, com as apostas de cada evento): 63 idênticas byte a byte; a outra é o controle do PnL sem `now`.

| Pedido ao cache da Polymarket | Antes | Depois |
|---|---|---|
| 120 eventos, sem recorte | 164 ms | 96 ms |
| 60 eventos, 30 com recorte | 102 ms | 18 ms |
| 400 eventos, recorte das apostas | 153 ms | 136 ms |

### 6. Entregue no app instalado, e a memória do WebView2

- **Instalação:** `cargo tauri build` em `src-tauri` e o instalador com `/S /UPDATE` (o modo de atualização não cria atalhos novos). Antes, uma cópia do banco com `copiar_banco.py`. Os números estão em [No app instalado](#no-app-instalado).
- **Janela minimizada:** `lib.rs::memoria_ao_minimizar` pede ao WebView2 o nível de memória baixo (`ICoreWebView2_19::SetMemoryUsageTargetLevel`, pelo `with_webview` do Tauri e o crate `webview2-com`) quando a janela minimiza, e o normal quando volta. A página não é suspensa: timers e sincronização seguem.
  - Medido: 1 minuto minimizado, o WebView2 caiu 18 MB (GPU 103 → 87 MB, página 88 → 86 MB).
  - Ao restaurar, a página era a mesma (`medir_app.mjs conferir`).
  - Não houve controle no mesmo build sem o recurso, então parte da queda pode ser do próprio Windows.
- **`--disable-gpu-rasterization`:** sem diferença na memória da GPU (ver [O que não vale a pena mexer](#o-que-não-vale-a-pena-mexer)).

## Como validar

### Método

1. **Trabalhe numa cópia do banco, nunca no original.**
   - Copie com `ferramentas/validacao/copiar_banco.py`.
   - Ao trocar uma cópia por outra, apague antes o `-wal` e o `-shm` dela. Um `-wal` velho ao lado de um `.db` novo é aplicado por cima na abertura e corrompe a cópia ("freelist leaf count too big", rotas com 500).
   - Apague as cópias, os JSONL e as capturas no fim: são dados reais da conta.
2. **Compare o Rust novo com a referência.**
   - O teste ignorado `api::tests::despejar_rotas_para_comparacao` roda uma lista de rotas sobre um banco e grava as respostas em JSONL.
   - Variáveis: `BETTY_COMPARAR_DB`, `BETTY_COMPARAR_URLS` (uma linha por rota: `/rota?x`, ou `POST /rota arquivo.json`), `BETTY_COMPARAR_SAIDA`, e opcionalmente `BETTY_COMPARAR_CONFIG`.
   - Rode antes e depois da mudança (ou nos dois bancos) e compare os corpos.
   - Para as rotas de leitura, `rotas_leitura.py` monta a lista e os corpos dos POST a partir da cópia, e `comparar_rotas.py` compara os dois JSONL. Use uma cópia nova do banco em cada lado: algumas rotas gravam (estado salvo, métricas pendentes). Para ter o código antigo sem outra pasta (o Kaspersky só libera `src-tauri\target`), `git stash push -- src`, rode, e `git stash pop`.
3. **Compare o que a interface deriva dos dados, com o JS real.**
   - Os oráculos carregam `ui/*.js` num `node:vm`, com DOM falso (Proxy) e `localStorage` em memória, e chamam as funções reais: `indiceHistoricoFechado`, `tituloExibicao`, `classificarAposta`, `resolverDataInicio`, `getTsResultadoPnL`...
   - Troque valores entre o vm e o Node como texto JSON (`deepStrictEqual` falha entre realms).
   - **Sempre rode um controle negativo:** estrague os dados de propósito e confirme que o oráculo acusa.
4. **Ver o app sem mexer no instalado.** O bloqueio de instância única usa o identificador, então o build de teste precisa de outro:

   ```powershell
   $env:CARGO_PROFILE_DEV_OPT_LEVEL = "2"
   $env:TAURI_CONFIG = '{"identifier":"com.betty.desktop.teste"}'
   cargo build   # em src-tauri
   $env:BETTY_DATA_DIR = "<pasta com a cópia do banco>"
   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-gpu-rasterization --remote-debugging-port=9333"
   $env:BETTY_LOG_API = "1"   # tempo de cada rota no stderr
   .\target\debug\betty.exe
   ```

   - `BETTY_DATA_DIR` e as outras variáveis `BETTY_*` só valem em build de depuração.
   - Depois, apague `%LOCALAPPDATA%\com.betty.desktop.teste`.
   - Para medir rápido, compile os testes otimizados sem LTO: `CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 cargo test --release --lib --no-run`.

### Ferramentas (`ferramentas/validacao/`)

| Arquivo | Para quê |
|---|---|
| `copiar_banco.py` | Cópia consistente do banco (backup do SQLite, origem só leitura). |
| `raio_x_banco.py` | Linhas, tamanho do ledger e dos caches, espaço livre. |
| `ler_localstorage.py` | Chaves e tamanhos do localStorage de um perfil WebView2 (lê o LevelDB). |
| `teste_abertura.mjs` | Abertura com dados salvos, backend falso: sincronização ok, falha, primeira abertura. |
| `teste_versoes.mjs` | Versões do índice no SQLite e cache de fechadas em memória. |
| `observar_abertura.mjs` | Linha do tempo da abertura e capturas pelo CDP (porta da depuração remota). |
| `oraculo_compactacao.mjs` | Modelo de oráculo: compara o que a interface deriva de dois bancos. |
| `oraculo_recorte.mjs` + `preparar_meta.py` | Modelo de oráculo para respostas de rota (evento inteiro x recortado). |
| `oraculo_categorias.mjs` | Casos e respostas do `classificarAposta()` real para `categorias::tests::oraculo_categorias`, a partir de um despejo com `GET /api/history/closed`. |
| `rotas_leitura.py` + `comparar_rotas.py` | Lista de 64 rotas de leitura para o `despejar_rotas_para_comparacao` e comparação byte a byte dos dois despejos, com o tempo de cada rota. |
| `medir_app.mjs` | Pelo CDP, sem capturas: linha do tempo da abertura (tela, dados salvos, confirmação), compilação e execução de cada script numa recarga, e um marcador para saber se a página recarregou. |
| `medir_memoria.ps1` | Memória privada do `betty.exe` (e o pico) e de cada processo do WebView2 dele, por papel (página, GPU, rede...). |

Para medir o app instalado pelo CDP, abra-o com `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` com os mesmos flags de `lib.rs` mais `--remote-debugging-port=<porta>`, só no processo que abre o app. A sessão de 2026-11-27 seguiu esta ordem: medir o build antigo aberto, copiar o banco, fechar (`CloseMainWindow`), instalar, abrir duas vezes medindo, `scripts`, minimizar 1 minuto, abrir sem o flag da GPU e, no fim, abrir normalmente. São várias aberturas seguidas: combine antes com quem usa a máquina.
Rode `teste_abertura.mjs` e `teste_versoes.mjs` a cada mudança no `dashboard.js`:

```powershell
node ferramentas/validacao/teste_abertura.mjs ui
node ferramentas/validacao/teste_versoes.mjs ui
```

E o Rust: `cargo test --lib` e `cargo clippy --all-targets` (hoje só há 3 avisos, antigos, em `tracked.rs`).

## O que falta

Em ordem de prioridade. Cada item diz o problema, a evidência, a proposta, os cuidados, como validar e quando está pronto.

### 1. Menos `serde_json::Value` nos caminhos das casas

- **Problema:**
  - O `serde_json` está com `arbitrary_precision` e `preserve_order` no app inteiro. É necessário para o ledger sair byte a byte igual, mas faz cada número virar texto alocado e cada objeto um `IndexMap`.
  - As respostas da Polymarket (`poly::buscar_linhas`, `normalizar`) e da Kalshi são lidas inteiras como `Value`.
- **Já feito:** `poly::ler_tudo` media o tamanho da lista serializando-a inteira; agora `js::tamanho_json` conta os mesmos bytes sem guardar o texto.
- **Proposta:** structs com `#[derive(Deserialize)]` nos campos usados, ou `RawValue` (`json_bruto`) onde o JSON é repassado sem mudança.
- **Antes de fazer:** falta um gravador de respostas das casas. As linhas são normalizadas, filtradas, juntadas com o cache de activity no SQLite e paginadas para a interface, então mudar a leitura exige comparar byte a byte com respostas reais gravadas. Hoje não há como reproduzir uma sessão sem rede.
- **Validar:** respostas das rotas das casas idênticas (`despejar_rotas_para_comparacao` contra respostas gravadas de uma sessão real) e oráculos existentes (`tracked::tests::oraculo_predict`, `kalshi_dados::tests::oraculo_titulos`, `ledger::tests::oraculo_ledger`).

### 2. Memória do processo Rust parado

- **Problema:** 211 MB privados parado no build antigo. Ainda não foi medido depois das mudanças.
- **Proposta:** medir primeiro (item 1). Se continuar alto, os suspeitos são:
  - o `mimalloc` segurando memória depois de picos (testar `mi_option_purge_decommits` ou `mi_collect`, ou comparar com o alocador do sistema);
  - o cache de páginas das conexões de leitura (até 8 MB cada, em `db::abrir_leitura`). As páginas lidas por `mmap` não entram nele nem na memória privada;
  - os snapshots da Polymarket (até 64 MB serializados, mais em `Value`);
  - as liquidações da Kalshi guardadas em memória (`EstadoKalshi::liquidacoes`, cerca de 1.944 linhas).
- **Cuidado:** `EstadoKalshi::liquidacoes` e `revisados`, e as agendas de `EstadoTracked`, mudam comportamento (cache da sessão, agenda vencida usada como reserva). Não basta expirar por tempo.

### 3. Sincronização inteira no Rust (o item grande)

- **Problema:**
  - Toda a sincronização está no `dashboard.js` (`carregarAba`). A interface baixa as listas das casas pelo backend, enriquece, concilia, compara com o ledger, envia o delta e reindexa, com centenas de idas e voltas pelo `betty://` e o histórico inteiro no heap do JS.
  - Enquanto ela grava o índice em lotes, a interface trava as leituras (`iniciarSincronizacaoIndiceHistorico`). Os dados salvos contornam isso só na abertura.
- **O que existe hoje, na ordem (`carregarAba`):**
  1. Em paralelo:
     - `fetchPosicoesAbertas()`: posições, mais datas e esporte por `buscarDatasInicioPorSlug()` e `resolverDataInicio()`;
     - `fetchPosicoesFechadas()`: fechadas, `enriquecerFechadasPolymarketIncremental()`, `mesclarPerdasNasFechadas()` e `corrigirPnlPorFluxo()` com `buscarFluxoPolymarket()`;
     - Kalshi: `fetchKalshiAbertos()`, `fetchKalshiFechados()` e saldo;
     - saldos e `POST /api/tracked/:casa/sync` (Predict e SX já sincronizam no backend).
  2. `reconciliarSnapshotPolymarket()` e `reconciliarSnapshotKalshi()`. A cada 10 min, `reconciliarPolymarketPorActivity()`.
  3. `normalizarDatasPolymarketPersistidas()`, depois `carregarAssinaturasLedgerFechadas()` e `linhasAlteradasNoLedger()` (assinatura `assinaturaFonteLedger()`).
  4. `salvarLedgerFechadas()` (`POST .../delta`, merge no Rust) e depois `sincronizarIndiceHistoricoFechado()`. Ela chama `indiceHistoricoFechado()` para cada linha, com categoria (`classificarAposta()`), título (`tituloExibicao()`) e datas, e grava em lotes de 200.
- **Já portado para o Rust, com oráculo:**
  - `titles.rs` (o `market-title-utils.js`, incluindo `exactMarket`);
  - `breakdown.rs`, `audit.rs` (`pnlTimestamp`, `protectClosedDate`) e `ledger.rs` (`ledgerMerge`);
  - `kalshi_dados.rs` (títulos, datas e modalidade da Kalshi), `poly_datas.rs` e `tracked.rs`;
  - `categorias.rs`: `classificarAposta()` e todos os auxiliares (`sinaisDaAposta`, `normalizarEsporte`, `categoriaPorTema`, `esportePorTicker`, `esportePorTiposMercado`, `slugDeConfronto`, `metadadoDizEsporte`, `esportePorConfronto`, `temaDoMercadoProprio`). Ainda não é chamado pelas rotas.
    - Oráculo `categorias::tests::oraculo_categorias`, com os casos de `oraculo_categorias.mjs`: 27.709 casos iguais ao JS. São as 4.232 apostas do ledger como a interface as recebe, seis variantes de cada (sem `_sportMeta`, título de outra, outra casa, sem campos de esporte, slugs de outra, metadado de outra) e 36 casos de borda, 4 deles com exceção no JS, que o Rust devolve como `Err`.
    - Controle negativo: 555 respostas estragadas de propósito, 555 acusadas.
    - Cuidados que o porte já resolve e que valem para os próximos: `\s` e `trim()` do JS (incluem U+FEFF, não U+0085), `.` do JS (não casa `\r`, U+2028 e U+2029), `String([a, b])` = `"a,b"`, e o espalhamento `...(x || [])`, que percorre um texto letra a letra e lança exceção num número ou objeto.
- **Só existe no JS (precisa portar com oráculo):**
  - `normalizarNomeMercadoPolymarket()` e `tituloExibicao()`;
  - `resolverDataInicio()` com `metaDatasGamma()` e `extrairMetaEsporteEvento()`;
  - `corrigirPnlPorFluxo()`, `mesclarPerdasNasFechadas()`, as reconciliações e `indiceHistoricoFechado()`.
- **Proposta:**
  1. Portar as funções puras uma a uma, cada uma com oráculo contra o JS real sobre todas as apostas do ledger. É o mesmo método já usado em `titles.rs` (918.675 casos de título idênticos).
  2. Uma tarefa em segundo plano no Rust faz a sincronização inteira. A gravação do ledger e do índice vira **uma transação**: as leituras em WAL veem ou o estado antigo ou o novo, e a trava de leitura da interface deixa de ser necessária.
  3. O progresso vai para a interface por evento do Tauri ou por uma rota de estado. A interface só lê páginas prontas.
  4. Manter o pipeline do JS atrás de uma chave até o Rust dar o mesmo índice, o mesmo ledger e os mesmos números em várias sincronizações reais. Depois, apagar.
- **Validar:** ledger e índice byte a byte iguais entre o pipeline do JS e o do Rust na mesma cópia de banco e com as mesmas respostas das casas. Gravar as respostas de uma sessão real e reproduzir.

### 4. Sincronização incremental (depois do item 3)

- **Problema:** cada sincronização baixa o histórico inteiro de fechadas da Polymarket (3 páginas de 1.000 no v2) e conferem tudo de novo. A activity da Polymarket (`poly::atividade_incremental`) e as liquidações da Kalshi (`kalshi::liquidacoes_incrementais`) já são incrementais.
- **Proposta:** as fechadas vêm em ordem de `TIMESTAMP DESC`. Parar quando aparecerem K linhas seguidas que já estão no ledger com a mesma assinatura. Fazer a varredura completa em segundo plano de tempos em tempos (por exemplo 1 vez por dia ou na abertura).
- **Antes de fazer:** confirmar com dados reais se uma aposta antiga pode mudar (`realizedPnl`, resgate) sem subir para o topo da ordenação. Para isso, compare duas varreduras completas em dias diferentes.
  - O ledger sozinho não responde: o `updated_at` também muda quando o próprio app enriquece a aposta (datas recuperadas, `_sportMeta`, o reenvio depois de mudar o formato do metadado), e não há histórico das versões.
  - O caminho é gravar a lista inteira de fechadas (`GET` das fechadas da Polymarket, pelas rotas do app) em dois dias diferentes e comparar, aposta por aposta, quem mudou e em que posição estava.

## O que não vale a pena mexer

- **Perfil de release:** já tem `lto = true`, `codegen-units = 1`, `panic = "abort"` e `strip`.
- **Tamanho do binário:** cerca de 11 MB.
- **Consultas comuns ao SQLite:** já são de 3 a 18 ms.
- **Timer de 1 s da barra lateral (`tickAtualizado`):** só troca o DOM quando o texto muda.
- **Dividir o `dashboard.js` por tela:** medido no app instalado (`medir_app.mjs scripts`, uma recarga com rastreamento): o `dashboard.js` leva 0,1 ms para compilar e 1,4 ms para executar, e os outros seis scripts somam menos de 1 ms. A página fica pronta em 0,16–0,45 s desde a navegação. Separar por tela não teria o que ganhar.
- **Tirar o `--disable-gpu-rasterization`:** o processo de GPU ficou com 100–103 MB com o flag e 101 MB sem ele. Não há memória a ganhar, e o flag existe pela qualidade do SVG.

## Armadilhas conhecidas

- **Assinaturas da fonte:** `assinaturaFonteLedger()` inclui o `_sportMeta`. Mudar o formato do metadado faz as apostas afetadas serem reenviadas uma vez ao ledger (o merge mantém o que está gravado). É esperado.
- **A data do PnL no backend:** `ledger::salvar_indice` calcula `pnl_ts` sozinho, com `audit::pnl_timestamp`. O `pnlTs` que a interface manda é ignorado.
- **Cota do localStorage:** fica em torno de 5 a 10 MB. Qualquer cache grande no navegador falha calado. Prefira o SQLite.
- **`dadosFinanceirosConfirmados()`:** a regra de só mostrar números confirmados só cede nos dados salvos da abertura. Não use cache do navegador como fonte de números financeiros.
- **429 da Kalshi:** os endpoints públicos da Kalshi respondem 429 em rajadas. `net::get_json` já tenta de novo em 429/503. Ao comparar duas execuções, dê um intervalo entre elas.
- **`mmap` e `VACUUM` no Windows:** com outra conexão do processo mapeando o arquivo, o truncamento depois do `VACUUM` não acontece e não dá erro: o arquivo fica do mesmo tamanho (medido: 40 MB antes e depois, contra menos de 4 MB sem o `mmap`). Por isso as conexões de leitura só ligam o `mmap` depois da manutenção (`Db::mapear_leitura`). Qualquer `VACUUM` novo fora dela precisa desligar o `mmap` delas antes (`PRAGMA mmap_size=0` em cada uma). O teste `db::tests::vacuum_encolhe_o_arquivo_com_as_leituras_abertas` falha se elas estiverem mapeadas.
- **`ler` só lê:** uma gravação dentro de `Db::ler` falha com `SQLITE_READONLY`. Uma rota de leitura que passe a gravar (como `garantir_metricas`) precisa da etapa na conexão de escrita antes, como `api::completar_metricas`.
- **PnL e o relógio:** a série do `/pnl` termina no instante do pedido. Para comparar duas execuções, passe `now=` fixo.
- **Finais de linha:** o repositório usa `core.autocrlf=true`. Scripts que regravam arquivos podem trocar LF por CRLF na cópia de trabalho; confira antes do commit.
