#!/usr/bin/env python3
"""Built-app MCP smoke using disposable Dovecot, a TLS SMTP sink and test-only users.
Run: cargo build -p dakia-server --bin dakia-server --example mcp_fixture
     python3 scripts/mcp-disposable.py [--serve]
All listeners bind loopback. No owner configuration or credentials are read.
"""
import argparse, base64, concurrent.futures, email.message, http.client, http.server
import imaplib, json, os, pathlib, signal, socket, socketserver, sqlite3, ssl, subprocess, threading, time, urllib.request, urllib.error, uuid
ROOT=pathlib.Path(__file__).resolve().parents[1]
IMAGE='dovecot/dovecot@sha256:547d28f9a893c93a1d2f6dc164880cd7bfa74009106dfa46b36a6a2ee636d921'
def run(*args,**kw): return subprocess.run(args,check=True,text=True,capture_output=True,**kw).stdout.strip()
def port():
 with socket.socket() as s: s.bind(('127.0.0.1',0)); return s.getsockname()[1]
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--serve',action='store_true');args=parser.parse_args()
 directory=ROOT/'target'/('mcp-disposable-'+uuid.uuid4().hex[:10]);directory.mkdir()
 name=directory.name;app=None;services=[];results=[];counts={'accepted':0,'attempts':0,'mode':'accept'}
 signal.signal(signal.SIGTERM,lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
 run('openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout',str(directory/'key.pem'),'-out',str(directory/'cert.pem'),'-days','2','-subj','/CN=localhost','-addext','subjectAltName=DNS:localhost','-addext','extendedKeyUsage=serverAuth')
 context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.load_cert_chain(directory/'cert.pem',directory/'key.pem')
 class SMTP(socketserver.StreamRequestHandler):
  def handle(self):
   self.wfile.write(b'220 synthetic ESMTP\r\n');self.wfile.flush()
   while line:=self.rfile.readline():
    cmd=line.upper()
    if cmd.startswith(b'EHLO'): reply=b'250-localhost\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME\r\n'
    elif cmd.startswith(b'AUTH'): reply=b'235 authenticated\r\n'
    elif cmd.startswith(b'MAIL'): counts['attempts']+=1;reply=b'250 sender\r\n'
    elif cmd.startswith(b'RCPT'): reply=b'550 synthetic refusal\r\n' if counts['mode']=='refuse' else b'250 recipient\r\n'
    elif cmd.startswith(b'DATA'):
     self.wfile.write(b'354 body\r\n');self.wfile.flush()
     while True:
      body=self.rfile.readline()
      if body in (b'.\r\n',b''):break
     counts['accepted']+=1
     if counts['mode']=='unknown':return
     reply=b'250 queued\r\n'
    elif cmd.startswith(b'QUIT'):self.wfile.write(b'221 bye\r\n');return
    else:reply=b'250 ok\r\n'
    self.wfile.write(reply);self.wfile.flush()
 smtp=socketserver.ThreadingTCPServer(('127.0.0.1',0),SMTP);smtp.daemon_threads=True;smtp.socket=context.wrap_socket(smtp.socket,server_side=True);threading.Thread(target=smtp.serve_forever,daemon=True).start();services.append(smtp)
 smtpport=smtp.server_address[1];appport=port();webport=port();origin=f'https://localhost:{webport}';base=f'http://127.0.0.1:{appport}'
 opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
 def api_http(path,body=None,method=None,token=None,bearer=None):
  headers={'Origin':origin,'Accept':'application/json, text/event-stream'}
  if token:headers.update({'Cookie':f'__Host-dakia={token}','X-CSRF-Token':'fixture-csrf'})
  if bearer:headers['Authorization']='Bearer '+bearer
  if body is not None:headers['Content-Type']='application/json';body=json.dumps(body).encode()
  req=urllib.request.Request(base+path,data=body,headers=headers,method=method)
  try:
   with opener.open(req,timeout=20) as response:return json.loads(response.read() or b'null')
  except urllib.error.HTTPError as e:
   return {'http_status':e.code,'response':e.read().decode()}
 def rpc(token,tool,arguments=None):
  method='tools/list' if tool=='tools/list' else 'tools/call'
  v=api_http('/mcp',{'jsonrpc':'2.0','id':1,'method':method,'params':{} if method=='tools/list' else {'name':tool,'arguments':arguments or {}}},bearer=token)
  assert 'result' in v,v
  return v['result'] if method=='tools/list' else v['result']['structuredContent']
 def check(label):results.append(label);print('PASS '+label,flush=True)
 def start():
  env=dict(os.environ,DAKIA_ORIGIN=origin,DAKIA_DATA_DIR=str(directory/'instance/data'),DAKIA_CREDENTIAL_KEY_FILE=str(directory/'instance/key'),DAKIA_OIDC_ISSUER='https://fixture.invalid',DAKIA_OIDC_CLIENT_ID='fixture',DAKIA_OIDC_SECRET_FILE=str(directory/'instance/oidc'),DAKIA_LISTEN=f'127.0.0.1:{appport}',DAKIA_WEB_ROOT=str(ROOT/'apps/web/dist'),DAKIA_RELEASE_CHECKS='false',DAKIA_DATABASE_URL='',DAKIA_SEARCH_DATABASE_URL='',DAKIA_SEARCH_MODEL_DIR=str(directory/'absent-model'),SSL_CERT_FILE=str(directory/'cert.pem'),DAKIA_NODE=str(pathlib.Path(os.environ.get('NODE_BINARY','/home/arsalan/.hermes/tools/node-26.12.1-linux-x64/bin/node'))))
  process=subprocess.Popen([str(ROOT/'target/debug/dakia-server')],cwd=ROOT,env=env,stdout=(directory/'server.log').open('ab'),stderr=subprocess.STDOUT)
  for _ in range(150):
   if process.poll() is not None:raise RuntimeError('server stopped; inspect '+str(directory/'server.log'))
   try:
    if api_http('/healthz').get('status')=='ok':return process
   except OSError:pass
   time.sleep(.1)
  raise RuntimeError('server startup timeout')
 try:
  run('docker','run','-d','--name',name,'-p','127.0.0.1::993','--mount',f'type=bind,src={ROOT}/crates/mail-provider/fixtures/dovecot/dovecot.conf,dst=/etc/dovecot/dovecot.conf,readonly','--mount',f'type=bind,src={directory},dst=/fixture,readonly',IMAGE)
  state=json.loads(run('docker','inspect',name))[0];imapport=int(state['NetworkSettings']['Ports']['993/tcp'][0]['HostPort'])
  run('docker','exec',name,'doveadm','service','status','imap-login')
  client_context=ssl.create_default_context(cafile=str(directory/'cert.pem'))
  for user in ('alice','bob'):
   mail=imaplib.IMAP4_SSL('localhost',imapport,ssl_context=client_context);mail.login(user,'fixture-only-password')
   for folder in ('Sent','Trash','Archive','Disposable'):mail.create(folder)
   for n in range(1,7):
    m=email.message.EmailMessage();m['From']='sender@example.invalid';m['To']=f'{user}@example.invalid';m['Subject']=f'Synthetic MCP message {n}';m['Message-ID']=f'<fixture-{user}-{n}@example.invalid>';m.set_content('Synthetic body audit sentinel');m.add_attachment(b'synthetic attachment',maintype='application',subtype='octet-stream',filename='fixture.bin');mail.append('INBOX',None,None,m.as_bytes())
   mail.logout()
  fixture=json.loads(run(str(ROOT/'target/debug/examples/mcp_fixture'),str(directory/'instance'),str(imapport),str(smtpport)))
  alice,bob=fixture['accounts'];account=alice['account_id'];app=start()
  class Proxy(http.server.BaseHTTPRequestHandler):
   def log_message(self,*_):pass
   def do_GET(self):self.forward()
   def do_POST(self):self.forward()
   def do_PUT(self):self.forward()
   def do_DELETE(self):self.forward()
   def forward(self):
    body=self.rfile.read(int(self.headers.get('Content-Length','0')));conn=http.client.HTTPConnection('127.0.0.1',appport,timeout=30)
    try:
     headers={k:v for k,v in self.headers.items() if k.lower() not in ('host','connection')};headers['Host']=f'localhost:{webport}'
     conn.request(self.command,self.path,body,headers);r=conn.getresponse();data=r.read();self.send_response(r.status)
     for k,v in r.getheaders():
      if k.lower() not in ('transfer-encoding','connection','content-length'):self.send_header(k,v)
     self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
    except (OSError,TimeoutError):pass
    finally:conn.close()
  proxy=http.server.ThreadingHTTPServer(('127.0.0.1',webport),Proxy);proxy.daemon_threads=True;proxy.socket=context.wrap_socket(proxy.socket,server_side=True);threading.Thread(target=proxy.serve_forever,daemon=True).start();services.append(proxy)
  created=api_http('/api/agent-connections',{'name':'Disposable assistant'},'POST','alice-fixture');token=created['token'];cid=created['connection']['id'];path='/api/agent-connections/'+cid
  assert created['connection']['allow_send'] is False and created['connection']['allow_delete'] is False
  check('built HTTP app creates default-off private connection')
  for send in (False,True):
   for delete in (False,True):
    api_http(path,{'name':'Disposable assistant','allow_send':send,'allow_delete':delete},'PUT','alice-fixture')
    names={v['name'] for v in rpc(token,'tools/list')['tools']};assert ('send_draft' in names)==send
    for name in ('trash_message','delete_message','empty_trash','delete_folder','delete_draft'):
     assert (name in names)==delete
     if not delete:assert rpc(token,name)['error']=='permission_denied'
    if not send:assert rpc(token,'send_draft')['error']=='permission_denied'
  check('four independent permission combinations, hidden tools and cached direct-call denial')
  assert rpc(token,'list_accounts')['rows'][0]['id']==account
  assert rpc(token,'read_draft',{'account_id':bob['account_id'],'draft_id':bob['draft_id']})['error']=='not_found'
  assert rpc(token,'list_drafts',{'account_id':account})['rows'][0]['id']==alice['draft_id']
  for bypass in ('request','batch','operation','move_message'):assert rpc(token,bypass,{'path':'/api/accounts','change':{'kind':'trash'}})['error']=='unknown_tool'
  check('all human drafts, cross-user isolation and generic/batch bypass rejection')
  folders=rpc(token,'list_folders',{'account_id':account});assert any(f['name']=='INBOX' for f in folders),folders
  page=rpc(token,'search_messages',{'account_id':account,'folder':'INBOX','query':'Synthetic'});assert page.get('rows'),page
  message=page['rows'][0];mid=message['id'];content=rpc(token,'read_message',{'account_id':account,'message_id':mid});assert 'Synthetic body' in content['text'],content
  part=content['attachments'][0];attachment=rpc(token,'get_attachment',{'account_id':account,'attachment_id':part['id']});assert base64.b64decode(attachment['base64'])==b'synthetic attachment',attachment
  check('real TLS IMAP folder discovery, search, message and attachment retrieval')
  api_http(path,{'name':'Disposable assistant','allow_send':False,'allow_delete':False},'PUT','alice-fixture')
  assert 'error' not in rpc(token,'read_thread',{'account_id':account,'message_id':mid})
  reply=rpc(token,'prepare_reply',{'account_id':account,'message_id':mid,'text':'Synthetic reply'});assert reply.get('in_reply_to'),reply
  forward=rpc(token,'prepare_forward',{'account_id':account,'message_id':mid,'to':[{'email':'recipient@example.invalid','name':None}],'text':'Synthetic forward'});assert forward.get('attachments'),forward
  forward['text']+=' edited';edited=rpc(token,'save_draft',{'account_id':account,'draft':forward});assert edited.get('revision',0)>forward['revision'],edited
  for action in ('read','unread','star','unstar','label','archive'):
   current=rpc(token,'read_message',{'account_id':account,'message_id':mid})['message']
   operation=rpc(token,'update_message',{'account_id':account,'message_id':mid,'request_id':'organise-'+action,'revision':current['revision'],'action':action,**({'label':'people'} if action=='label' else {})});assert operation.get('id'),operation
   for _ in range(150):
    state=rpc(token,'get_operation',{'account_id':account,'operation_id':operation['id']})
    if state['status'] in ('applied','failed'):break
    time.sleep(.1)
   assert state['status']=='applied',state
  check('read thread, reply and forward with attachment, draft edit and baseline organisation without send/delete permission')
  # Reject extra-field and mixed-effect mutation attempts rather than ignoring them.
  assert rpc(token,'update_message',{'account_id':account,'message_id':mid,'request_id':'bypass','revision':0,'action':'archive','change':{'kind':'trash'}})['error']=='invalid_arguments'
  assert rpc(token,'update_message',{'account_id':account,'message_id':mid,'request_id':'bypass','revision':0,'action':'trash'})['error']=='invalid_arguments'
  check('extra-field and mixed-effect mutation bypasses rejected')
  extra=api_http('/api/accounts',{'email':'extra@example.invalid','display_name':'New fixture account','provider':'imap','imap_host':'localhost','imap_port':imapport,'imap_tls':'implicit','smtp_host':'localhost','smtp_port':smtpport,'smtp_tls':'implicit','username':'alice','password':'fixture-only-password'},'POST','alice-fixture');assert extra.get('id'),extra
  ids={v['id'] for v in rpc(token,'list_accounts')['rows']};assert extra['id'] in ids and bob['account_id'] not in ids,ids
  check('newly connected fixture account is immediately visible only to owner')
  for name,draft in [('reply',reply),('forward',edited)]:
   assert rpc(token,'send_draft',{'account_id':account,'draft_id':draft['id'],'revision':draft['revision'],'request_id':'blocked-'+name})['error']=='permission_denied'
  check('reply and forward sends share the independent send gate')
  api_http(path,{'name':'Disposable assistant','allow_send':False,'allow_delete':True},'PUT','alice-fixture')
  trashmid=page['rows'][1]['id'];current=rpc(token,'read_message',{'account_id':account,'message_id':trashmid})['message']
  trashed=rpc(token,'trash_message',{'account_id':account,'message_id':trashmid,'request_id':'trash-fixture','revision':current['revision']});assert trashed.get('id'),trashed
  for _ in range(150):
   state=rpc(token,'get_operation',{'account_id':account,'operation_id':trashed['id']})
   if state['status'] in ('applied','failed'):break
   time.sleep(.1)
  assert state['status']=='applied',state
  emptied=rpc(token,'empty_trash',{'account_id':account,'message_ids':[trashmid],'request_id':'empty-fixture'});assert emptied.get('results',[{}])[0].get('outcome')=='confirmed',emptied
  assert rpc(token,'delete_draft',{'account_id':account,'draft_id':reply['id'],'revision':reply['revision']})['deleted'] is True
  check('Trash, explicit empty Trash and draft deletion work independently of sending')
  api_http(path,{'name':'Disposable assistant','allow_send':True,'allow_delete':True},'PUT','alice-fixture')
  sendargs={'account_id':account,'draft_id':alice['draft_id'],'revision':alice['revision'],'request_id':'once'}
  with concurrent.futures.ThreadPoolExecutor() as pool:sent=list(pool.map(lambda _:rpc(token,'send_draft',sendargs),range(4)))
  assert len({s['id'] for s in sent})==1,sent
  for _ in range(200):
   status=rpc(token,'get_send_status',{'request_id':'once'})
   if status['state'] in ('accepted','failed','delivery_unknown'):break
   time.sleep(.1)
  assert status['state']=='accepted',status;assert counts['accepted']==1,counts
  app.terminate();app.wait(timeout=10);app=start();assert rpc(token,'send_draft',sendargs)['id']==sent[0]['id'];time.sleep(1);assert counts['accepted']==1,counts
  check('concurrent duplicate sends and built-server restart preserve one SMTP acceptance')
  changed=dict(sendargs,revision=alice['revision']+1);assert rpc(token,'send_draft',changed)['error']=='revision_conflict'
  counts['mode']='unknown';unknown=dict(sendargs,request_id='unknown');rpc(token,'send_draft',unknown)
  for _ in range(200):
   status=rpc(token,'get_send_status',{'request_id':'unknown'})
   if status['state'] in ('accepted','failed','delivery_unknown'):break
   time.sleep(.1)
  assert status['state']=='delivery_unknown',status
  before=counts['attempts'];app.terminate();app.wait(timeout=10);app=start();rpc(token,'send_draft',unknown);time.sleep(6);assert counts['attempts']==before,counts
  check('lost SMTP acknowledgement remains unknown across restart without resend')
  counts['mode']='refuse';rpc(token,'send_draft',dict(sendargs,request_id='refused'))
  for _ in range(200):
   status=rpc(token,'get_send_status',{'request_id':'refused'})
   if status['state'] in ('accepted','failed','delivery_unknown'):break
   time.sleep(.1)
  assert status['state']=='failed',status;check('preaccept SMTP recipient rejection is failed, not unknown')
  deleted=rpc(token,'delete_folder',{'account_id':account,'folder':'Disposable','request_id':'delete-folder'});assert deleted.get('deleted'),deleted
  check('real disposable IMAP folder deletion with readback')
  permanent=rpc(token,'delete_message',{'account_id':account,'message_id':mid,'request_id':'delete-message'});assert permanent['results'][0]['outcome']=='confirmed',permanent
  check('real UID-scoped permanent deletion with readback')
  with sqlite3.connect(directory/'instance/data/control.sqlite') as db:
   audit=json.dumps(db.execute('SELECT * FROM agent_audit').fetchall());assert token not in audit and 'Synthetic private body' not in audit and 'fixture-only-password' not in audit and 'Authorization' not in audit
  assert 'Synthetic body audit sentinel' not in audit
  check('audit records contain no fixture bodies, credentials or bearer token')
  api_http(path,None,'DELETE','alice-fixture');response=api_http('/mcp',{'jsonrpc':'2.0','id':1,'method':'tools/list'},bearer=token);assert response['http_status']==401,response
  with sqlite3.connect(directory/'instance/data/control.sqlite') as db:
   assert db.execute("SELECT count(*) FROM agent_audit WHERE connection_id=? AND action='authentication' AND outcome='denied'",(cid,)).fetchone()[0]>=1
  check('revocation invalidates cached HTTP client immediately and records redacted denial')
  (directory/'results.json').write_text(json.dumps({'passed':results,'smtp':counts,'origin':origin,'data':'isolated synthetic only'},indent=2))
  (directory/'browser-state.json').write_text(json.dumps({'origin':origin,'cookie':'alice-fixture','directory':str(directory)}))
  print('Evidence: '+str(directory/'results.json'),flush=True)
  if args.serve:
   print('Browser fixture: '+origin,flush=True)
   threading.Event().wait()
 finally:
  if app and app.poll() is None:app.terminate();app.wait(timeout=10)
  for service in services:service.shutdown();service.server_close()
  subprocess.run(['docker','rm','-f',name],capture_output=True)
  print('Disposed task-owned services; evidence retained at '+str(directory),flush=True)
if __name__=='__main__':main()
