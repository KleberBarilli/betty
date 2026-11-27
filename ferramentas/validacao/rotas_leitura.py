"""Monta a lista de rotas de leitura (e os corpos dos POST) para o teste
`api::tests::despejar_rotas_para_comparacao`, a partir de uma cópia do banco.

    python -I rotas_leitura.py <banco.db> <pasta de saída>

Grava `rotas.txt` e os JSON dos POST na pasta. Rode o despejo antes e depois
da mudança, cada um numa cópia nova do banco, e compare com `comparar_rotas.py`.
Os corpos têm chaves reais da conta: apague a pasta no fim.
"""
import json
import os
import sqlite3
import sys

banco, pasta = sys.argv[1], sys.argv[2]
c = sqlite3.connect(f"file:{banco}?mode=ro", uri=True)
user = c.execute("SELECT id FROM users ORDER BY created_at LIMIT 1").fetchone()[0]

categorias = [r[0] for r in c.execute(
    "SELECT category, COUNT(*) n FROM closed_bets_index WHERE user_id=? GROUP BY category ORDER BY n DESC LIMIT 4", [user])]
meses = [r[0] for r in c.execute(
    "SELECT DISTINCT substr(pnl_ymd,1,7) m FROM closed_bets_index WHERE user_id=? AND pnl_ymd<>'' ORDER BY m DESC LIMIT 3", [user])]
chaves = c.execute("SELECT platform, bet_key FROM closed_bets_ledger WHERE user_id=? ORDER BY rowid", [user]).fetchall()
poly = [k for p, k in chaves if p == "polymarket"]
slugs = [r[0] for r in c.execute("SELECT slug FROM polymarket_metadata_cache ORDER BY updated_at DESC LIMIT 120")]

def gravar(nome, corpo):
    with open(os.path.join(pasta, nome), "w", encoding="utf-8") as f:
        json.dump(corpo, f)

gravar("lookup.json", {"entries": [{"platform": p, "betKey": k} for p, k in chaves[:300]] + [{"platform": "kalshi", "betKey": "nao-existe"}]})
gravar("metadata.json", {"keys": poly[:400]})
gravar("metadata_vazio.json", {"keys": []})
gravar("cache_slugs.json", {"slugs": slugs})
gravar("cache_need.json", {"slugs": slugs[:60], "need": {s: {"slugs": [], "assets": [], "conditions": []} for s in slugs[:30]}})

# Como na abertura: cada evento com apostas no ledger pede os mercados delas.
por_evento = {}
for (dj,) in c.execute("SELECT data_json FROM closed_bets_ledger WHERE user_id=? AND platform='polymarket' ORDER BY rowid", [user]):
    d = json.loads(dj)
    n = por_evento.setdefault(d.get("eventSlug") or d.get("slug"), {"assets": [], "conditions": [], "slugs": []})
    for lista, k in (("assets", "asset"), ("conditions", "conditionId"), ("slugs", "slug")):
        if d.get(k):
            n[lista].append(d[k])
com_apostas = [r[0] for r in c.execute("SELECT slug FROM polymarket_metadata_cache ORDER BY slug") if r[0] in por_evento][:400]
gravar("cache_apostas.json", {"slugs": com_apostas, "need": {s: por_evento[s] for s in com_apostas}})

# `now` fixo no PnL: sem ele, o último ponto da série leva o instante do pedido.
AGORA = "now=1795800000000"
rotas = [
    "/api/me",
    "/api/settings",
    "/api/history/closed/index/status",
    "/api/history/closed/index/pending",
    "/api/history/closed/index/pending?cursor=0",
    "/api/history/closed/index/pending?cursor=x",
    "/api/history/closed/layout",
    "/api/history/closed/layout?platform=kalshi",
    "/api/history/closed/stats",
    "/api/history/closed/stats?includeMarkets=1&includeSegments=1&includePrevious=1&start=2026-09-01&end=2026-09-30",
    "/api/history/closed/stats?platform=polymarket&includePrevious=1&start=2026-10-01&end=2026-10-31",
    "/api/history/closed/stats?oddsRange=2&title=vs",
    "/api/history/closed/calendar?month=bad",
]
rotas += [f"/api/history/closed/calendar?month={m}" for m in meses]
rotas += [f"/api/history/closed/calendar?month={m}&platform=kalshi" for m in meses[:1]]
for modo in ["accumulated", "daily", "monthly"]:
    for periodo in ["all", "7", "30"]:
        rotas.append(f"/api/history/closed/pnl?mode={modo}&period={periodo}&{AGORA}")
rotas.append("/api/history/closed/pnl?mode=daily&period=all")
rotas.append("/api/history/closed/pnl?mode=daily&period=stats&startTs=1780000000000&endTs=1795000000000&platform=polymarket")
for pagina in [1, 2, 7, 50]:
    rotas.append(f"/api/history/closed/query?page={pagina}&pageSize=50")
for sort, d in [("data", "asc"), ("titulo", "desc"), ("odd", "asc"), ("stake", "desc"), ("pnl", "asc")]:
    rotas.append(f"/api/history/closed/query?sort={sort}&dir={d}&pageSize=20&layout=1")
for cat in categorias:
    rotas.append(f"/api/history/closed/query?category={cat}&pageSize=50")
    rotas.append(f"/api/history/closed/stats?category={cat}")
rotas += [
    "/api/history/closed/query?platform=kalshi&title=vs&pageSize=50",
    "/api/history/closed/query?day=2026-10-12&pageSize=50",
    # Estado salvo: sem snapshot cai na conexão normal; com ele, sai do estado salvo.
    "/api/history/closed/stats?snapshot=1",
    "POST /api/history/snapshot",
    "/api/history/closed/stats?snapshot=1&includeMarkets=1",
    "/api/history/closed/query?snapshot=1&page=3&pageSize=50",
    "/api/history/closed/layout?snapshot=1",
    f"/api/history/closed/pnl?snapshot=1&mode=daily&period=all&{AGORA}",
    "/api/history/closed/calendar?snapshot=1&month=" + (meses[0] if meses else "2026-10"),
    "/api/history/closed/index/status?snapshot=1",
    "DELETE /api/history/snapshot",
    "POST /api/history/closed/lookup lookup.json",
    "POST /api/history/closed/metadata metadata.json",
    "POST /api/history/closed/metadata metadata_vazio.json",
    "POST /api/polymarket-metadata/cache cache_slugs.json",
    "POST /api/polymarket-metadata/cache cache_need.json",
    "POST /api/polymarket-metadata/cache cache_apostas.json",
    "/api/history/closed",
    "/api/history/closed/export",
]
with open(os.path.join(pasta, "rotas.txt"), "w", encoding="utf-8") as f:
    f.write("\n".join(rotas) + "\n")
print(f"{len(rotas)} rotas; {len(chaves)} apostas, {len(poly)} da Polymarket, {len(slugs)} slugs")
