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

Processo do app instalado (build antigo), parado depois de abrir:

| Processo | Memória privada |
|---|---|
| `betty.exe` (Rust) | 211 MB, com pico de 1,1 GB na abertura |
| WebView2: página | 178 MB |
| WebView2: GPU | 124 MB |
| WebView2: browser, rede, storage, crashpad | 34 + 11 + 11 + 3 MB |

Abertura medida num build de teste com cópia do banco:
- **0 a 3,9 s:** tela de carregamento. Desses, 3,7 s são da compactação única do banco, que só acontece uma vez.
- **3,9 s:** os dados salvos aparecem.
- **11,5 s:** a sincronização confirma os números.

A sincronização é dominada pela rede: activity da Polymarket 4,5 s, fechadas da Polymarket 3,4 s, liquidações da Kalshi 2,3 s, sync da Predict 1,4 s.

> O `betty.exe` instalado ainda é o build antigo. Os números "depois" vieram de
> cópias do banco e de um build de teste. Medir no app instalado é o item 1 de
> [O que falta](#o-que-falta).

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

### 4. Leituras do SQLite em paralelo

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

## Como validar

### Método

1. **Trabalhe numa cópia do banco, nunca no original.**
   - Copie com `ferramentas/validacao/copiar_banco.py`.
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
| `rotas_leitura.py` + `comparar_rotas.py` | Lista de 63 rotas de leitura para o `despejar_rotas_para_comparacao` e comparação byte a byte dos dois despejos, com o tempo de cada rota. |

Rode `teste_abertura.mjs` e `teste_versoes.mjs` a cada mudança no `dashboard.js`:

```powershell
node ferramentas/validacao/teste_abertura.mjs ui
node ferramentas/validacao/teste_versoes.mjs ui
```

E o Rust: `cargo test --lib` e `cargo clippy --all-targets` (hoje só há 3 avisos, antigos, em `tracked.rs`).

## O que falta

Em ordem de prioridade. Cada item diz o problema, a evidência, a proposta, os cuidados, como validar e quando está pronto.

### 1. Entregar e medir no app instalado (fazer primeiro)

- **Problema:** nada do que já foi feito chegou ao app que o usuário usa.
- **Proposta:**
  1. Gerar o instalador (`cargo tauri build` em `src-tauri`) e instalar.
  2. A primeira abertura roda a compactação única, cerca de 4 s com a tela de carregamento. A partir da segunda, os dados salvos aparecem logo.
  3. Medir:
     - memória privada do `betty.exe` e de cada processo do WebView2, parado depois de abrir;
     - pico do `betty.exe` (`PeakPagedMemorySize64`);
     - tempo até os dados salvos e até a confirmação, em mais de uma abertura.
- **Pronto quando:** os números estiverem nesta página, ao lado dos de partida.

### 2. Memória do WebView2

- **Problema:** página com 178 MB e GPU com 124 MB, mesmo minimizado.
- **Proposta:**
  - Quando a janela for minimizada ou escondida, chamar `ICoreWebView2_19::put_MemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW)` e voltar ao normal ao restaurar. O acesso é por `webview.with_webview(...)` no Tauri, com o crate `webview2-com`.
  - Medir também o efeito de `--disable-gpu-rasterization` (em `lib.rs`) na memória do processo de GPU. Ele existe por qualidade do SVG (serrilhado na GPU); não tirar sem comparar o visual.
- **Validar:** memória privada antes e depois, minimizado por 1 minuto; o app volta sem recarregar a página.
- **Pronto quando:** a queda estiver medida e anotada aqui.

### 3. Cache por evento sem ler o evento inteiro

- **Problema:** `poly::cache_metadados` ainda transforma o evento inteiro em `serde_json::Value` antes de recortar, 300 a 400 ms para cerca de 225 eventos. Desde as leituras em paralelo ela não segura mais a conexão de escrita, mas ocupa uma das duas de leitura. A tabela `polymarket_metadata_cache` tem 65 MB.
- **Proposta (uma das duas):**
  - (a) Ler só o primeiro nível com `RawValue` e decodificar só `dates.markets[slug]` e os mercados de `sport.markets` que casam.
  - (b) Guardar o cache por mercado (uma tabela `slug` + `market_slug`) e consultar só as linhas necessárias.
- **Na mesma linha:** `kalshi_metadata_cache` tem 40 MB, e boa parte é duplicada dentro de cada linha (`_markets` = `markets` e `_milestones` = `_milestone`, cerca de 17 MB). Antes de tirar a duplicata, confirmar em `kalshi.rs` e `kalshi_dados.rs` quem lê cada chave.
- **Validar:** respostas da rota idênticas às de hoje (com e sem `need`), com o `oraculo_recorte.mjs`.

### 4. Menos `serde_json::Value` nos caminhos das casas

- **Problema:**
  - O `serde_json` está com `arbitrary_precision` e `preserve_order` no app inteiro. É necessário para o ledger sair byte a byte igual, mas faz cada número virar texto alocado e cada objeto um `IndexMap`.
  - As respostas da Polymarket (`poly::buscar_linhas`, `normalizar`) e da Kalshi são lidas inteiras como `Value`.
  - `poly::ler_tudo` serializa a lista inteira só para medir o tamanho (`js::para_vec(&linhas).len()`).
- **Proposta:**
  - Structs com `#[derive(Deserialize)]` nos campos usados, ou `RawValue` onde o JSON é repassado sem mudança.
  - Estimar o tamanho sem serializar.
- **Validar:** respostas das rotas das casas idênticas (`despejar_rotas_para_comparacao` contra respostas gravadas de uma sessão real) e oráculos existentes (`tracked::tests::oraculo_predict`, `kalshi_dados::tests::oraculo_titulos`, `ledger::tests::oraculo_ledger`).

### 5. Memória do processo Rust parado

- **Problema:** 211 MB privados parado no build antigo. Ainda não foi medido depois das mudanças.
- **Proposta:** medir primeiro (item 1). Se continuar alto, os suspeitos são:
  - o `mimalloc` segurando memória depois de picos (testar `mi_option_purge_decommits` ou `mi_collect`, ou comparar com o alocador do sistema);
  - o cache de páginas das conexões de leitura (até 8 MB cada, em `db::abrir_leitura`). As páginas lidas por `mmap` não entram nele nem na memória privada;
  - os snapshots da Polymarket (até 64 MB serializados, mais em `Value`);
  - as liquidações da Kalshi guardadas em memória (`EstadoKalshi::liquidacoes`, cerca de 1.944 linhas).
- **Cuidado:** `EstadoKalshi::liquidacoes` e `revisados`, e as agendas de `EstadoTracked`, mudam comportamento (cache da sessão, agenda vencida usada como reserva). Não basta expirar por tempo.

### 6. Dividir o `dashboard.js` por tela (medir antes)

- **Problema possível:** são 7.700 linhas e 380 KB lidas e compiladas de uma vez na abertura.
- **Proposta:**
  1. Medir antes, pelo CDP (`Performance` ou `Profiler`), quanto tempo vai em compilar e avaliar o script.
  2. Se for relevante, separar por tela (estatísticas, calendário, configurações) com `import()` dinâmico. O CSP já permite scripts da própria origem.
- **Cuidado:** o arquivo usa muito estado global e funções chamadas por nome (`data-on-click`, em `acoes.js`).

### 7. Sincronização inteira no Rust (o item grande)

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
  - `kalshi_dados.rs` (títulos, datas e modalidade da Kalshi), `poly_datas.rs` e `tracked.rs`.
- **Só existe no JS (precisa portar com oráculo):**
  - `classificarAposta()` e auxiliares (`sinaisDaAposta`, `esportePorTicker`, `esportePorTiposMercado`...);
  - `normalizarNomeMercadoPolymarket()` e `tituloExibicao()`;
  - `resolverDataInicio()` com `metaDatasGamma()` e `extrairMetaEsporteEvento()`;
  - `corrigirPnlPorFluxo()`, `mesclarPerdasNasFechadas()`, as reconciliações e `indiceHistoricoFechado()`.
- **Proposta:**
  1. Portar as funções puras uma a uma, cada uma com oráculo contra o JS real sobre todas as apostas do ledger. É o mesmo método já usado em `titles.rs` (918.675 casos de título idênticos).
  2. Uma tarefa em segundo plano no Rust faz a sincronização inteira. A gravação do ledger e do índice vira **uma transação**: as leituras em WAL veem ou o estado antigo ou o novo, e a trava de leitura da interface deixa de ser necessária.
  3. O progresso vai para a interface por evento do Tauri ou por uma rota de estado. A interface só lê páginas prontas.
  4. Manter o pipeline do JS atrás de uma chave até o Rust dar o mesmo índice, o mesmo ledger e os mesmos números em várias sincronizações reais. Depois, apagar.
- **Validar:** ledger e índice byte a byte iguais entre o pipeline do JS e o do Rust na mesma cópia de banco e com as mesmas respostas das casas. Gravar as respostas de uma sessão real e reproduzir.

### 8. Sincronização incremental (depois do item 7)

- **Problema:** cada sincronização baixa o histórico inteiro de fechadas da Polymarket (3 páginas de 1.000 no v2) e conferem tudo de novo. A activity da Polymarket (`poly::atividade_incremental`) e as liquidações da Kalshi (`kalshi::liquidacoes_incrementais`) já são incrementais.
- **Proposta:** as fechadas vêm em ordem de `TIMESTAMP DESC`. Parar quando aparecerem K linhas seguidas que já estão no ledger com a mesma assinatura. Fazer a varredura completa em segundo plano de tempos em tempos (por exemplo 1 vez por dia ou na abertura).
- **Antes de fazer:** confirmar com dados reais se uma aposta antiga pode mudar (`realizedPnl`, resgate) sem subir para o topo da ordenação. Para isso, compare duas varreduras completas em dias diferentes.

## O que não vale a pena mexer

- **Perfil de release:** já tem `lto = true`, `codegen-units = 1`, `panic = "abort"` e `strip`.
- **Tamanho do binário:** cerca de 11 MB.
- **Consultas comuns ao SQLite:** já são de 3 a 18 ms.
- **Timer de 1 s da barra lateral (`tickAtualizado`):** só troca o DOM quando o texto muda.

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
