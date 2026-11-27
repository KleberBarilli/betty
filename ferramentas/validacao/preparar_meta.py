"""Pedidos do cache de metadados por evento para o oráculo do recorte.
A: todas as apostas do ledger de cada evento pedem; B: só a primeira pede;
C: eventos sem apostas no ledger, com uma aposta sintética do 1º mercado."""
import sqlite3, sys, json, os, collections

db, pasta = sys.argv[1], sys.argv[2]
c = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
linhas = collections.defaultdict(list)
for (dj,) in c.execute("SELECT data_json FROM closed_bets_ledger WHERE platform='polymarket' ORDER BY rowid"):
    d = json.loads(dj)
    linhas[d.get('eventSlug') or d.get('slug')].append(d)
eventos = [r[0] for r in c.execute("SELECT slug FROM polymarket_metadata_cache ORDER BY slug")]
com = [e for e in eventos if e in linhas]
sem = [e for e in eventos if e not in linhas]


def necessidade(rows):
    assets, conditions, slugs = [], [], []
    for d in rows:
        for v in (d.get('asset'), d.get('assetId'), d.get('tokenId')):
            if v is not None and str(v).strip(): assets.append(str(v))
        for v in (d.get('conditionId'), d.get('condition_id')):
            if v is not None and str(v).strip(): conditions.append(str(v))
        if d.get('slug') is not None and str(d.get('slug')).strip(): slugs.append(str(d['slug']))
    return {'assets': assets, 'conditions': conditions, 'slugs': slugs}


sinteticas = {}
for e in sem:
    dado = json.loads(c.execute("SELECT data_json FROM polymarket_metadata_cache WHERE slug=?", (e,)).fetchone()[0])
    m = next((m for m in (dado.get('sport') or {}).get('markets') or [] if isinstance(m, dict) and m.get('tokenIds')), None)
    if m:
        sinteticas[e] = [{'_platform': 'polymarket', 'eventSlug': e, 'slug': m.get('slug'), 'conditionId': m.get('conditionId'),
                          'asset': m['tokenIds'][0], 'outcome': 'Yes', 'outcomeIndex': 0, 'title': m.get('question') or e,
                          'realizedPnl': 1, 'initialValue': 1, 'timestamp': 1790000000}]

cenarios = {
    'A': {e: linhas[e] for e in com},
    'B': {e: linhas[e][:1] for e in com},
    'C': sinteticas,
}
urls, pedidos = [], {}
for nome, mapa in cenarios.items():
    chaves = list(mapa)
    pedidos[nome] = {e: mapa[e] for e in chaves}
    for i in range(0, len(chaves), 400):
        lote = chaves[i:i + 400]
        for tipo, corpo in (('cheio', {'slugs': lote}), ('recorte', {'slugs': lote, 'need': {e: necessidade(mapa[e]) for e in lote}})):
            arq = f'{nome}_{tipo}_{i // 400}.json'
            json.dump(corpo, open(os.path.join(pasta, arq), 'w'))
            urls.append(f'POST /api/polymarket-metadata/cache {arq}')
open(os.path.join(pasta, 'urls_meta_evento.txt'), 'w').write('\n'.join(urls) + '\n')
json.dump(pedidos, open(os.path.join(pasta, 'pedidos.json'), 'w', encoding='utf-8'), ensure_ascii=False)
print({k: len(v) for k, v in cenarios.items()}, 'eventos por cenário;', len(urls), 'consultas')
