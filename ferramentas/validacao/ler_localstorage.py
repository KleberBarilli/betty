"""Lista as chaves do localStorage de um perfil WebView2 (LevelDB), só leitura.

    python -I ler_localstorage.py "<perfil>\EBWebView\Default\Local Storage\leveldb" [chave saída.json]

Mostra nome, tamanho e se a chave está apagada; nunca imprime valores. Com
`chave` e `saída.json`, grava o valor daquela chave no arquivo (dados reais:
apague depois). Lê os .ldb (blocos snappy) e o .log do LevelDB.
O perfil do app instalado fica em `%LOCALAPPDATA%\com.betty.desktop`."""
import os, struct, sys, glob

pasta = sys.argv[1]


def varint(b, p):
    r = s = 0
    while True:
        c = b[p]; p += 1
        r |= (c & 0x7f) << s
        if c < 0x80:
            return r, p
        s += 7


def snappy(b):
    n, p = varint(b, 0)
    out = bytearray()
    while p < len(b):
        t = b[p]; p += 1
        k = t & 3
        if k == 0:
            ln = t >> 2
            if ln >= 60:
                nb = ln - 59
                ln = int.from_bytes(b[p:p + nb], 'little'); p += nb
            ln += 1
            out += b[p:p + ln]; p += ln
            continue
        if k == 1:
            ln = ((t >> 2) & 7) + 4; off = ((t >> 5) << 8) | b[p]; p += 1
        elif k == 2:
            ln = (t >> 2) + 1; off = int.from_bytes(b[p:p + 2], 'little'); p += 2
        else:
            ln = (t >> 2) + 1; off = int.from_bytes(b[p:p + 4], 'little'); p += 4
        for _ in range(ln):
            out.append(out[-off])
    assert len(out) == n
    return bytes(out)


def bloco(d, off, size):
    raw = d[off:off + size]
    if d[off + size] == 1:
        raw = snappy(raw)
    nrest = struct.unpack('<I', raw[-4:])[0]
    fim = len(raw) - 4 - 4 * nrest
    p, ant = 0, b''
    while p < fim:
        sh, p = varint(raw, p); ns, p = varint(raw, p); vl, p = varint(raw, p)
        k = ant[:sh] + raw[p:p + ns]; p += ns
        v = raw[p:p + vl]; p += vl
        ant = k
        yield k, v


entradas = {}  # chave -> (seq, apagada, tamanho)
valores = {}
DESEJADA = sys.argv[2].encode() if len(sys.argv) > 2 else None


def registrar(k, seq, apagada, tam, v=None):
    if k not in entradas or entradas[k][0] < seq:
        entradas[k] = (seq, apagada, tam)
        if DESEJADA and k.endswith(DESEJADA): valores[k] = v


for f in glob.glob(os.path.join(pasta, '*.ldb')):
    d = open(f, 'rb').read()
    rodape = d[-48:]
    p = 0
    _, p = varint(rodape, p); _, p = varint(rodape, p)
    io, p = varint(rodape, p); isz, p = varint(rodape, p)
    for _, h in bloco(d, io, isz):
        bo, q = varint(h, 0); bs, q = varint(h, q)
        for k, v in bloco(d, bo, bs):
            trailer = int.from_bytes(k[-8:], 'little')
            registrar(k[:-8], trailer >> 8, (trailer & 0xff) == 0, len(v), v)

for f in glob.glob(os.path.join(pasta, '*.log')):
    d = open(f, 'rb').read()
    registros, atual, p = [], b'', 0
    while p + 7 <= len(d):
        if (p % 32768) + 7 > 32768:
            p += 32768 - (p % 32768); continue
        ln = int.from_bytes(d[p + 4:p + 6], 'little'); tp = d[p + 6]
        if tp == 0 and ln == 0:
            p += 32768 - (p % 32768); continue
        frag = d[p + 7:p + 7 + ln]; p += 7 + ln
        if tp == 1: registros.append(frag)
        elif tp == 2: atual = frag
        elif tp == 3: atual += frag
        elif tp == 4: registros.append(atual + frag); atual = b''
    for r in registros:
        seq = int.from_bytes(r[:8], 'little'); n = int.from_bytes(r[8:12], 'little'); q = 12
        for i in range(n):
            tag = r[q]; q += 1
            kl, q = varint(r, q); k = r[q:q + kl]; q += kl
            if tag == 1:
                vl, q = varint(r, q); val = r[q:q + vl]; q += vl
                registrar(k, seq + i, False, vl, val)
            else:
                registrar(k, seq + i, True, 0)

for k, (seq, apagada, tam) in sorted(entradas.items()):
    if not k.startswith(b'_http://betty.localhost'):
        continue
    nome = k.split(b'\x00', 1)[1]
    nome = nome[1:].decode('latin-1') if nome[:1] == b'\x01' else nome[1:].decode('utf-16-le', 'replace')
    print(f"{'APAGADA ' if apagada else ''}{nome:48s} {tam/1048576:8.2f} MB")

if DESEJADA:
    for k, v in valores.items():
        texto = v[1:].decode('latin-1') if v[:1] == b'\x01' else v[1:].decode('utf-16-le')
        open(sys.argv[3], 'w', encoding='utf-8').write(texto)
