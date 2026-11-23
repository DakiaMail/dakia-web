// Official SDK protocol and OAuth harness. Not an authenticated ChatGPT/Claude session.
// npm install --prefix target/mcp-client --no-save @modelcontextprotocol/sdk@1.32.2
// NODE_EXTRA_CA_CERTS=<fixture>/cert.pem node scripts/mcp-sdk.mjs <fixture>/browser-state.json
import { Client } from '../target/mcp-client/node_modules/@modelcontextprotocol/sdk/dist/esm/client/index.js';
import { StreamableHTTPClientTransport } from '../target/mcp-client/node_modules/@modelcontextprotocol/sdk/dist/esm/client/streamableHttp.js';
import { auth } from '../target/mcp-client/node_modules/@modelcontextprotocol/sdk/dist/esm/client/auth.js';
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
const state=JSON.parse(readFileSync(process.argv[2],'utf8')), origin=state.origin, endpoint=origin+'/mcp';
const headers={ Cookie: '__Host-dakia='+state.cookie, Origin: origin, 'X-CSRF-Token': 'fixture-csrf', 'Content-Type': 'application/json' };
const manage=async (path,method='GET',body) => {const r=await fetch(origin+'/api/agent-connections'+path,{method,headers,body:body===undefined?undefined:JSON.stringify(body)});assert.ok(r.ok,`management HTTP ${r.status}`);return r.status===204?null:r.json();};
const created=await manage('','POST',{name:'Official SDK fixture'}), id=created.connection.id;
const passed=[]; let bearer,oauth;
try {
  bearer=new Client({name:'dakia-official-sdk-fixture',version:'1'});
  await bearer.connect(new StreamableHTTPClientTransport(new URL(endpoint),{requestInit:{headers:{Authorization:'Bearer '+created.token}}}));
  assert.equal(bearer.getServerVersion().name,'Dakia');
  let tools=await bearer.listTools();assert.ok(tools.tools.some(t=>t.name==='list_accounts'));assert.ok(!tools.tools.some(t=>t.name==='send_draft'));
  const accounts=await bearer.callTool({name:'list_accounts',arguments:{}});assert.ok(accounts.structuredContent.rows.length>=1);
  passed.push('Official SDK initializes stateless Streamable HTTP, lists tools, and calls list_accounts with connection bearer token');
  let information,tokens,verifier,authorization;
  const provider={redirectUrl:'https://client.example.invalid/sdk-callback',clientMetadata:{client_name:'Official SDK synthetic client',redirect_uris:['https://client.example.invalid/sdk-callback'],token_endpoint_auth_method:'none',grant_types:['authorization_code','refresh_token'],response_types:['code']},clientInformation:()=>information,saveClientInformation:v=>{information=v},tokens:()=>tokens,saveTokens:v=>{tokens=v},saveCodeVerifier:v=>{verifier=v},codeVerifier:()=>verifier,redirectToAuthorization:url=>{authorization=url},state:()=> 'fixture-state'};
  assert.equal(await auth(provider,{serverUrl:endpoint}),'REDIRECT');
  assert.equal(authorization.origin,origin);assert.equal(authorization.searchParams.get('resource'),endpoint);
  const form=new URLSearchParams(authorization.search);form.set('csrf','fixture-csrf');form.set('connection_id',id);form.set('decision','approve');
  const approval=await fetch(origin+'/oauth/authorize',{method:'POST',headers:{Cookie:headers.Cookie,Origin:origin,'Content-Type':'application/x-www-form-urlencoded'},body:form,redirect:'manual'});
  assert.equal(approval.status,303);const callback=new URL(approval.headers.get('location'));assert.equal(callback.searchParams.get('state'),'fixture-state');
  assert.equal(await auth(provider,{serverUrl:endpoint,authorizationCode:callback.searchParams.get('code')}),'AUTHORIZED');
  assert.ok(tokens.access_token);assert.ok(tokens.refresh_token);
  oauth=new Client({name:'dakia-oauth-sdk-fixture',version:'1'});
  await oauth.connect(new StreamableHTTPClientTransport(new URL(endpoint),{authProvider:provider}));
  tools=await oauth.listTools();assert.ok(!tools.tools.some(t=>t.name==='send_draft'));
  await manage('/'+id,'PUT',{name:'Official SDK fixture',allow_send:true,allow_delete:false});
  tools=await oauth.listTools();assert.ok(tools.tools.some(t=>t.name==='send_draft'));assert.ok(!tools.tools.some(t=>t.name==='delete_draft'));
  await manage('/'+id,'PUT',{name:'Official SDK fixture',allow_send:false,allow_delete:false});
  const denied=await oauth.callTool({name:'send_draft',arguments:{}});assert.equal(denied.isError,true);assert.equal(denied.structuredContent.error,'permission_denied');
  passed.push('Official SDK discovers OAuth metadata, dynamically registers, completes resource-bound PKCE, connects, observes edits and receives cached-tool denial');
  const previous=tokens.access_token;tokens={...tokens,access_token:'expired-synthetic-token'};
  assert.equal(await auth(provider,{serverUrl:endpoint}),'AUTHORIZED');assert.notEqual(tokens.access_token,previous);
  passed.push('Official SDK refresh-token exchange rotates credentials');
  await manage('/'+id,'DELETE');
  const revoked=await fetch(endpoint,{method:'POST',headers:{Authorization:'Bearer '+tokens.access_token,'Content-Type':'application/json'},body:JSON.stringify({jsonrpc:'2.0',id:1,method:'tools/list'})});assert.equal(revoked.status,401);
  passed.push('OAuth access is invalid immediately after connection revocation');
  const evidence={sdk:JSON.parse(readFileSync('target/mcp-client/node_modules/@modelcontextprotocol/sdk/package.json')).version,passed,limits:'Synthetic SDK harness only. No authenticated ChatGPT or Claude Code session.'};
  writeFileSync(dirname(process.argv[2])+'/sdk-results.json',JSON.stringify(evidence,null,2));console.log(JSON.stringify(evidence,null,2));
} finally {await bearer?.close();await oauth?.close();await manage('/'+id,'DELETE');}
