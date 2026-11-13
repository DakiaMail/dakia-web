import sqlite3,json,re,time,statistics
from pathlib import Path
root=Path(__file__).resolve().parents[3]
c=sqlite3.connect(':memory:')
for p in sorted((root/'crates/storage/migrations/account').glob('*.sql')):
 if p.name.startswith('0022'):continue
 c.executescript(p.read_text())
rows=[]
for i in range(100000):
 m={'id':f'm{i:06}','thread_id':f't{i:06}','provider_id':'opaque','folder_ids':['INBOX'],'received_at':f'{i:09}','starred':i%10000==0,'classification':{'attention':[{'label':'Action'}] if i%10000==1 else [],'attention_override':None}}
 rows.append((m['id'],0,json.dumps(m)))
c.executemany('INSERT INTO messages(id,revision,document) VALUES(?,?,?)',rows)
c.executemany("INSERT INTO snoozes VALUES(?, '2999-01-01','')",[(f't{i:06}',) for i in range(0,100000,10000)])
classification=re.search(r'CLASSIFICATION_SQL: &str = "(.*?)";', (root/'crates/storage/src/classification.rs').read_text(),re.S)[1]
def query(file,marker,attention=False):
 s=(root/file).read_text();s=s[s.index(marker):];q=re.search(r'(?:r#)?"(SELECT .*?)"#?[,\n]',s,re.S)[1];q=q.replace('{CLASSIFICATION_SQL}',classification)
 if '{attention}' in q:q=q.replace('{attention}',"AND (json_extract(m.document,'$.classification.attention_override')=1 OR json_array_length(m.document,'$.classification.attention')>0)" if attention else '')
 return q
star=query('crates/storage/src/cache.rs','pub async fn starred_rows_filtered')
attention=query('crates/storage/src/inbox.rs','pub async fn inbox_rows_filtered',True)
snooze=query('crates/storage/src/snoozes.rs','pub async fn snoozed_rows_filtered')
filters=json.dumps({'attention':True})
queries=[('Starred',star,['{}',None,None,None,None,None]),('Attention',attention,[filters,'2026-11-13',None,None,None,None,None]),('Snoozed',snooze,['{}','2026-11-13',None,None,None,None,None])]
original={}
for phase in ['before','after']:
 if phase=='after':c.executescript((root/'crates/storage/migrations/account/0022_sparse_mail_views.sql').read_text())
 for label,q,args in queries:
  if phase=='before' and label=='Attention':q=q.replace("AND (json_extract(m.document,'$.classification.attention_override')=1 OR json_array_length(m.document,'$.classification.attention')>0)",'')
  times=[]
  for _ in range(3):
   t=time.perf_counter();result=c.execute(q,args).fetchall();times.append((time.perf_counter()-t)*1000)
  if phase=='before': original[label]=result
  else:
   assert result==original[label], label
   expected={'Starred':'messages_starred_received_order','Attention':'messages_attention_received_order','Snoozed':'messages_thread_received_order'}[label]
   assert any(expected in row[3] for row in c.execute('EXPLAIN QUERY PLAN '+q,args)), label
  print(phase,label,len(result),round(statistics.median(times),2),'ms',c.execute('EXPLAIN QUERY PLAN '+q,args).fetchall(),flush=True)
