"""Deterministic, read-only IMAP fixture. Never accepts mailbox mutations."""
import argparse, asyncio, json, re, ssl, time
from collections import Counter

p = argparse.ArgumentParser()
p.add_argument('--port', type=int, default=1993)
p.add_argument('--count', type=int, default=50000)
p.add_argument('--delay-ms', type=float, default=0)
p.add_argument('--cert', required=True)
p.add_argument('--key', required=True)
p.add_argument('--output', required=True)
a = p.parse_args()
stats = dict(connections=0, commands=Counter(), request_bytes=0, response_bytes=0, header_rows=0, body_rows=0, rejected=[])

def header(uid):
    return (f'From: Sender {uid % 100} <sender{uid % 100}@example.test>\r\nTo: fixture@example.test\r\nSubject: Benchmark message {uid}\r\nDate: Thu, 05 Nov 2026 12:00:00 +0000\r\nMessage-ID: <bench-{uid}@example.test>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n').encode()
def body(uid):
    return (f'Synthetic message {uid}. Search token body{uid}.\r\n' + 'A deterministic mailbox benchmark. ' * 48 + '\r\n').encode()

def ids(spec):
    result = set()
    for part in spec.split(','):
        pair = [a.count if v == '*' else int(v) for v in part.split(':')]
        lo, hi = min(pair), max(pair)
        result.update(range(max(1, lo), min(a.count, hi)+1))
    return sorted(result)

def save():
    with open(a.output, 'w') as f: json.dump(stats, f, indent=2)

async def client(r, w):
    stats['connections'] += 1
    async def send(b):
        if isinstance(b, str): b = b.encode()
        stats['response_bytes'] += len(b)
        w.write(b)
        await w.drain()
    await send('* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] synthetic benchmark\r\n')
    try:
        while line := await r.readline():
            stats['request_bytes'] += len(line)
            tag, cmd = line.decode().rstrip().split(' ', 1)
            upper = cmd.upper()
            name = ' '.join(upper.split()[:2]) if upper.startswith('UID ') else upper.split()[0]
            stats['commands'][name] += 1
            if a.delay_ms: await asyncio.sleep(a.delay_ms / 1000)
            response = b''
            if upper.startswith('LOGIN '): pass
            elif upper.startswith('AUTHENTICATE PLAIN'):
                if len(cmd.split()) == 2:
                    await send('+ \r\n'); await r.readline()
            elif upper == 'CAPABILITY': response = b'* CAPABILITY IMAP4rev1 AUTH=PLAIN\r\n'
            elif upper.startswith(('LIST ', 'LSUB ')): response = b'* LIST (\\HasNoChildren) "/" "INBOX"\r\n'
            elif upper.startswith(('SELECT ', 'EXAMINE ')):
                response = f'* {a.count} EXISTS\r\n* 0 RECENT\r\n* FLAGS (\\Seen \\Flagged)\r\n* OK [UIDVALIDITY 17] epoch\r\n* OK [UIDNEXT {a.count+1}] next\r\n'.encode()
            elif upper.startswith('STATUS '): response = f'* STATUS "INBOX" (MESSAGES {a.count} UNSEEN {a.count} UIDVALIDITY 17 UIDNEXT {a.count+1})\r\n'.encode()
            elif upper.startswith('UID SEARCH '):
                selected = set() if re.search(r'\b(?:FLAGGED|SEEN)\b', upper) else set(range(1, a.count+1))
                for spec in re.findall(r'\bUID ([\d*:,]+)', upper): selected.intersection_update(ids(spec))
                response = ('* SEARCH ' + ' '.join(map(str, sorted(selected))) + '\r\n').encode()
            elif upper.startswith(('UID FETCH ', 'FETCH ')):
                spec, fields = cmd.split(' ', 3)[2:] if upper.startswith('UID ') else cmd.split(' ', 2)[1:]
                pieces = []
                for uid in ids(spec):
                    h, b = header(uid), body(uid)
                    attrs = [f'UID {uid}', 'FLAGS ()', f'RFC822.SIZE {len(h)+len(b)}', 'INTERNALDATE "05-Nov-2026 12:00:00 +0000"']
                    fup = fields.upper()
                    if 'BODY' in fup or 'RFC822)' in fup:
                        match = re.search(r'BODY(?:\.PEEK)?\[([^]]*)\](?:<(\d+)\.(\d+)>)?', fields, re.I)
                        section = match[1] if match else ''
                        data = h if 'HEADER' in section.upper() else b if section.upper() == 'TEXT' else h+b
                        if 'HEADER.FIELDS' in section.upper():
                            wanted = re.search(r'\(([^)]+)\)', section)[1].upper().split()
                            data = b'\r\n'.join(l for l in h.split(b'\r\n') if l.split(b':')[0].decode().upper() in wanted)+b'\r\n\r\n'
                        origin = ''
                        if match and match[2]:
                            start, length = int(match[2]), int(match[3]); data = data[start:start+length]; origin = f'<{start}>'
                        stats['header_rows' if 'HEADER' in section.upper() else 'body_rows'] += 1
                        attrs.append(f'BODY[{section}]{origin} {{{len(data)}}}\r\n')
                        pieces.append(f'* {uid} FETCH ('.encode() + ' '.join(attrs).encode() + data + b')\r\n')
                    else: pieces.append(f'* {uid} FETCH ({" ".join(attrs)})\r\n'.encode())
                response = b''.join(pieces)
            elif upper == 'LOGOUT':
                await send(f'* BYE done\r\n{tag} OK logout\r\n'); break
            elif upper in ('NOOP', 'CLOSE', 'UNSELECT'): pass
            else:
                stats['rejected'].append(cmd); await send(f'{tag} BAD unsupported\r\n'); save(); continue
            await send(response + f'{tag} OK done\r\n'.encode())
            save()
    except (ConnectionError, asyncio.IncompleteReadError): pass
    finally:
        save(); w.close()

async def main():
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(a.cert, a.key)
    server = await asyncio.start_server(client, '0.0.0.0', a.port, ssl=ctx)
    print(f'ready port={a.port} messages={a.count}', flush=True)
    async with server: await server.serve_forever()
asyncio.run(main())
