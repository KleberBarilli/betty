"""Copia o banco do Betty para um arquivo de teste, sem tocar no original.

Usa a API de backup do SQLite com o original aberto só para leitura, então a
cópia é consistente mesmo com o app aberto (WAL).

    python -I copiar_banco.py <destino.db> [origem.db]

Sem origem, copia `%APPDATA%\\com.betty.desktop\\betty.db`. Apague as cópias
quando terminar: elas têm os dados reais da conta.
"""
import os
import sqlite3
import sys
import time

destino = sys.argv[1]
origem = sys.argv[2] if len(sys.argv) > 2 else os.path.expandvars(r"%APPDATA%\com.betty.desktop\betty.db")
if os.path.exists(destino):
    os.remove(destino)
inicio = time.time()
fonte = sqlite3.connect(f"file:{origem}?mode=ro", uri=True)
copia = sqlite3.connect(destino)
fonte.backup(copia)
copia.close()
fonte.close()
print(f"copiado: {os.path.getsize(destino) / 1048576:.0f} MB em {time.time() - inicio:.1f} s")
