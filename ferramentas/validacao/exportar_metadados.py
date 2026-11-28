"""Exporta o cache de metadados da Polymarket de uma CÓPIA do banco para o
`oraculo_gamma.mjs`: uma linha `{"slug": ..., "data": {...}}` por evento.

    python -I exportar_metadados.py <copia.db> <saida.jsonl>

A saída tem dados reais da conta: apague no fim.
"""
import json
import sqlite3
import sys

origem, destino = sys.argv[1], sys.argv[2]
banco = sqlite3.connect(f"file:{origem}?mode=ro", uri=True)
with open(destino, "w", encoding="utf-8", newline="\n") as saida:
    n = 0
    for slug, dados in banco.execute("SELECT slug, data_json FROM polymarket_metadata_cache ORDER BY slug"):
        saida.write(json.dumps({"slug": slug, "data": json.loads(dados)}, ensure_ascii=False) + "\n")
        n += 1
banco.close()
print(f"{n} eventos")
