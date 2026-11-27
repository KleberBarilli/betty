"""Compara dois JSONL do `api::tests::despejar_rotas_para_comparacao`: status
e corpo de cada rota, byte a byte, sem o tempo (`ms`) e sem o instante da
exportação (`exportedAt`). Mostra também o tempo de cada rota nos dois.

    python -I comparar_rotas.py antes.jsonl depois.jsonl

Lê linha por linha: o ledger inteiro pode ter mais de 100 MB numa linha.
"""
import hashlib
import re
import sys

MS = re.compile(r'"ms":[0-9.eE+-]+,')
EXPORTADO = re.compile(r'"exportedAt":"[^"]*"')

def linhas(caminho):
    with open(caminho, encoding="utf-8") as f:
        for linha in f:
            cabeca = linha[:2000]
            url = re.search(r'"url":"((?:[^"\\]|\\.)*)"', cabeca).group(1)
            ms = float(re.search(r'"ms":([0-9.eE+-]+)', cabeca).group(1))
            corpo = EXPORTADO.sub('"exportedAt":""', MS.sub("", linha, count=1), count=1)
            yield url, ms, len(linha), hashlib.sha256(corpo.encode()).hexdigest()

a, b = list(linhas(sys.argv[1])), list(linhas(sys.argv[2]))
assert len(a) == len(b), (len(a), len(b))
iguais = 0
for (ua, ma, ta, ha), (ub, mb, _, hb) in zip(a, b):
    assert ua == ub, (ua, ub)
    igual = ha == hb
    iguais += igual
    print(f"{'=' if igual else 'DIFERENTE'} {ma:9.1f} ms -> {mb:9.1f} ms {ta / 1e6:8.2f} MB  {ua}")
print(f"\n{iguais}/{len(a)} rotas idênticas")
sys.exit(0 if iguais == len(a) else 1)
