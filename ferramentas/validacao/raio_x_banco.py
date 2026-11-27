"""Raio-x do banco: linhas por tabela, quanto o ledger e os caches ocupam e o
espaço livre do arquivo. Só lê.

    python -I raio_x_banco.py [banco.db]

Sem argumento, lê `%APPDATA%\\com.betty.desktop\\betty.db`.
"""
import os
import sqlite3
import sys

banco = sys.argv[1] if len(sys.argv) > 1 else os.path.expandvars(r"%APPDATA%\com.betty.desktop\betty.db")
c = sqlite3.connect(f"file:{banco}?mode=ro", uri=True)
q = lambda sql, *p: c.execute(sql, p).fetchall()
pagina = q("PRAGMA page_size")[0][0]
print(f"arquivo: {q('PRAGMA page_count')[0][0] * pagina / 1048576:.1f} MB, livre: {q('PRAGMA freelist_count')[0][0] * pagina / 1048576:.1f} MB")
print("--- linhas por tabela")
for (tabela,) in q("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"):
    print(f"  {tabela:32s} {q(f'SELECT COUNT(*) FROM {tabela}')[0][0]}")
print("--- ledger por casa: apostas, MB de data_json, média e maior (bytes)")
for linha in q("SELECT platform, COUNT(*), ROUND(SUM(LENGTH(data_json))/1048576.0,1), CAST(AVG(LENGTH(data_json)) AS INT), MAX(LENGTH(data_json)) FROM closed_bets_ledger GROUP BY platform"):
    print("  ", linha)
print("--- caches: linhas, MB, maior (bytes)")
for tabela in ["kalshi_settlement_cache", "kalshi_metadata_cache", "polymarket_metadata_cache", "polymarket_activity_cache"]:
    print(f"  {tabela:28s}", q(f"SELECT COUNT(*), ROUND(SUM(LENGTH(data_json))/1048576.0,1), MAX(LENGTH(data_json)) FROM {tabela}")[0])
print("--- kalshi_metadata_cache por tipo")
for linha in q("SELECT kind, COUNT(*), ROUND(SUM(LENGTH(data_json))/1048576.0,1) FROM kalshi_metadata_cache GROUP BY kind"):
    print("  ", linha)
print("--- manutenções feitas")
for linha in q("SELECT maintenance_key, completed_at FROM database_maintenance ORDER BY completed_at"):
    print("  ", linha)
