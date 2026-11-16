const { test } = require('node:test');
const assert = require('node:assert/strict');
const { validateProvisionalReceipt } = require('./mock-authority.cjs');

test('owner execution intent audit verifies stable signed scope without execution authority', () => {
  const { spawnSync } = require('node:child_process');
  const result = spawnSync('python3', ['-B', '-c', `
import sys,json,hashlib,copy,tempfile,pathlib,subprocess,os,uuid,inspect
from unittest.mock import patch
sys.path.insert(0,'scripts/auto-tag-eval-replay')
from importlib import import_module
m=import_module('audit-supervised')
assert callable(getattr(m,'owner_execution_intent_audit',None)), 'missing standalone owner execution intent auditor'
canonical=lambda v:json.dumps(v,ensure_ascii=False,sort_keys=True,separators=(',',':')).encode()
sha=lambda b:hashlib.sha256(b).hexdigest()
domain=b'soundrobe/owner-execution-intent/v1'+bytes([0])
env=dict(PATH=os.environ['PATH'],OPENSSL_CONF=os.devnull)
def openssl(*args):
    return subprocess.run(['openssl',*map(str,args)],env=env,check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=5).stdout
with tempfile.TemporaryDirectory(dir='.amp/in',prefix='intent-audit-') as temp:
    root=pathlib.Path(temp);private=root/'disposable-private.pem'
    openssl('genpkey','-algorithm','ED25519','-out',private)
    key=openssl('pkey','-in',private,'-pubout','-outform','DER')[-32:];trust=sha(key);now=1800000000
    proposal=dict(version=1,proposalId=str(uuid.uuid4()),executionKind='hostedSingleRequest',
        executableSha256=sha(b'not actual executable bytes'),membershipSha256='a'*64,nativePlanSha256='b'*64,
        requestSha256='c'*64,verifierSha256=trust,boundaryPricingSha256='d'*64,contextSha256='e'*64,
        sourcesAndTools=[dict(path='public/盧廣仲 😀.rs',sha256='f'*64),
            dict(path='.amp/in/artifacts/planned/native-supervision.json',runtime='nativeSupervision')])
    def sign(p=proposal,signing_key=private,prefix=domain,**claims):
        value=dict(version=1,proposalId=p['proposalId'],proposalSha256=sha(canonical(p)),executionKind=p['executionKind'],
            nonce=str(uuid.uuid4()),issuedUnixSeconds=now-600,expiresUnixSeconds=now+1200,
            hostedEstimateRiskAccepted=p['executionKind']=='hostedSingleRequest')
        value.update(claims)
        (root/'payload').write_bytes(prefix+canonical(value))
        openssl('pkeyutl','-sign','-rawin','-inkey',signing_key,'-in',root/'payload','-out',root/'signature')
        return dict(value,signature=list((root/'signature').read_bytes()))
    intent=sign();accepted=0;rejected=0
    def check(p=proposal,i=intent,k=key,t=trust,at=now):
        return m.owner_execution_intent_audit(p,i,k,t,at)
    expected=dict(version=1,signatureVerified=True,verifierSha256=trust,proposalId=proposal['proposalId'],
        proposalSha256=sha(canonical(proposal)),intentSha256=sha(canonical(intent)),executionKind='hostedSingleRequest',
        nonce=intent['nonce'],issuedUnixSeconds=now-600,expiresUnixSeconds=now+1200,hostedEstimateRiskAccepted=True,
        sourceBytesVerified=False,executionVerified=False,dispatchVerified=False,liveAuthority=False,userPresenceVerified=False,qualityCredit=0)
    before=copy.deepcopy((proposal,intent));real_run=subprocess.run;directories=[]
    def observed(args,**kwargs):
        assert args[:3]==['openssl','pkeyutl','-verify']
        assert kwargs['timeout']==5 and kwargs['stdin']==kwargs['stdout']==kwargs['stderr']==subprocess.DEVNULL
        assert kwargs['env']==env
        directory=pathlib.Path(args[args.index('-inkey')+1]).parent;directories.append(directory)
        assert set(p.name for p in directory.iterdir())=={'public.der','payload','signature'}
        assert (directory/'public.der').read_bytes()==bytes.fromhex('302a300506032b6570032100')+key
        assert (directory/'payload').read_bytes()==domain+canonical({k:v for k,v in intent.items() if k!='signature'})
        assert (directory/'signature').read_bytes()==bytes(intent['signature'])
        return real_run(args,**kwargs)
    with patch.object(m.subprocess,'run',side_effect=observed): assert check()==expected
    accepted+=1
    assert len(directories)==1 and not directories[0].exists()
    assert (proposal,intent)==before
    def deny(label,**kwargs):
        global rejected
        try: check(**kwargs)
        except AssertionError: rejected+=1
        else: raise AssertionError('accepted '+label)
    for failure in (OSError('unavailable'),subprocess.TimeoutExpired('openssl',5)):
        with patch.object(m.subprocess,'run',side_effect=failure): deny('verification unavailable')
    optimized=dict(vars(m))
    exec(compile(inspect.getsource(m.owner_execution_intent_audit),'optimized-intent-audit','exec',optimize=2),optimized)
    try: optimized['owner_execution_intent_audit'](proposal,intent,key,trust,now)
    except AssertionError: rejected+=1
    else: raise AssertionError('optimized Python accepted intent')
    for kind in ('offlineRehearsal','hostedSingleRequest'):
        p=dict(proposal,executionKind=kind,sourcesAndTools=[proposal['sourcesAndTools'][0]])
        for issued,expires in ((now,now+1),(now,now+1800),(now-1799,now+1)):
            assert check(p=p,i=sign(p,issuedUnixSeconds=issued,expiresUnixSeconds=expires))['signatureVerified'] is True
            accepted+=1
        deny('wrong risk',p=p,i=sign(p,hostedEstimateRiskAccepted=kind!='hostedSingleRequest'))
    for p in (dict(reversed(list(proposal.items()))),dict(proposal,sourcesAndTools=list(reversed(proposal['sourcesAndTools']))),
        dict(proposal,sourcesAndTools=[dict(path='native-supervision.json',runtime='nativeSupervision')]),
        dict(proposal,sourcesAndTools=[dict(path='public//./tool.rs/',sha256='a'*64)])):
        assert check(p=p,i=sign(p))['signatureVerified'] is True;accepted+=1
    deny('unsigned source order',p=dict(proposal,sourcesAndTools=list(reversed(proposal['sourcesAndTools']))))
    for field in proposal:
        p=copy.deepcopy(proposal);del p[field];deny('missing proposal '+field,p=p)
        p=copy.deepcopy(proposal);p[field]=None;deny('null proposal '+field,p=p)
    for field in intent:
        i=copy.deepcopy(intent);del i[field];deny('missing intent '+field,i=i)
        i=copy.deepcopy(intent);i[field]=None;deny('null intent '+field,i=i)
    for value in (None,[],True,dict(proposal,extra=1)): deny('proposal shape',p=value)
    for value in (None,[],True,dict(intent,extra=1)): deny('intent shape',i=value)
    for field,value in [('version',True),('version',1.0),('version',2),('proposalId',str(uuid.uuid1())),
        ('proposalId',proposal['proposalId'].upper()),('executionKind','hosted'),('executionKind',True)]:
        p=dict(proposal,**{field:value});deny('signed proposal type/identity',p=p,i=sign(p))
    for field in ('executableSha256','membershipSha256','nativePlanSha256','requestSha256','verifierSha256','boundaryPricingSha256','contextSha256'):
        for value in ('A'*64,'a'*63,True,[],0):
            p=dict(proposal,**{field:value});deny('signed digest '+field,p=p,i=sign(p))
        p=dict(proposal,**{field:'0'*64});deny('unsigned proposal drift '+field,p=p)
    for field,value in [('version',True),('version',1.0),('version',2),('proposalId',str(uuid.uuid4())),
        ('proposalSha256','0'*64),('executionKind','offlineRehearsal'),('hostedEstimateRiskAccepted',False),
        ('hostedEstimateRiskAccepted',1),('nonce',str(uuid.uuid1())),('nonce',intent['nonce'].upper()),('nonce',''),('extra',1)]:
        deny('signed invalid intent '+field,i=sign(**{field:value}))
    for field in ('issuedUnixSeconds','expiresUnixSeconds'):
        for value in (True,1.0,-1,2**64,'1800000000'):
            deny('signed time type '+field,i=sign(**{field:value}))
    for issued,expires in ((0,now+1),(now+1,now+2),(now,now),(now-1,now),(now-1800,now+1),(now,now+1801)):
        deny('original lifetime or validity',i=sign(issuedUnixSeconds=issued,expiresUnixSeconds=expires))
    for value in (True,1.0,-1,2**64,None,'now'): deny('audit time',at=value)
    for value in (None,b'',key[:-1],bytearray(key),list(key)): deny('key type',k=value)
    for value in (None,True,'0'*64,trust.upper()): deny('independent pin',t=value)
    k=bytes(32);p=dict(proposal,verifierSha256=sha(k));deny('zero key',p=p,i=sign(p),k=k,t=sha(k))
    for seed in (bytes.fromhex('9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60'),bytes([42])*32,bytes([43])*32):
        (root/'fixture-seed.der').write_bytes(bytes.fromhex('302e020100300506032b657004220420')+seed)
        fixture_private=root/'public-fixture-private.pem'
        openssl('pkey','-inform','DER','-in',root/'fixture-seed.der','-out',fixture_private)
        k=openssl('pkey','-in',fixture_private,'-pubout','-outform','DER')[-32:]
        p=dict(proposal,verifierSha256=sha(k))
        deny('validly signed fixture key',p=p,i=sign(p,signing_key=fixture_private),k=k,t=sha(k))
    other_private=root/'other-disposable-private.pem'
    openssl('genpkey','-algorithm','ED25519','-out',other_private)
    deny('wrong signer',i=sign(signing_key=other_private))
    for value in ('00',[],[0]*63,[0]*65,[True]*64,[-1]*64,[256]*64,[0.0]*64):
        deny('signature bytes',i=dict(intent,signature=value))
    i=copy.deepcopy(intent);i['signature'][0]^=1;deny('corrupt signature',i=i)
    deny('unsigned nonce',i=dict(intent,nonce=str(uuid.uuid4())))
    for prefix in (b'',domain[:-1],b'soundrobe/owner-approval/v1'+bytes([0])):
        deny('wrong signature domain',i=sign(prefix=prefix))
    for sources in ([],{},[proposal['sourcesAndTools'][0]]*2,[proposal['sourcesAndTools'][1]]*2,
        [dict(path='other.rs',runtime='nativeSupervision')],
        [dict(path='native-supervision.json',sha256='a'*64)],
        [dict(path='native-supervision.json',runtime='other')],
        [dict(path='native-supervision.json',runtime='nativeSupervision'),dict(path='other/native-supervision.json',runtime='nativeSupervision')],
        [dict(path='a.rs',sha256='a'*64,extra=True)],[dict(path='a.rs',sha256='A'*64)],
        [dict(path='a.rs',sha256=True)],[None],[dict(path='a.rs')],
        [dict(path=f'a/{n}.rs',sha256='a'*64) for n in range(4097)]):
        p=dict(proposal,sourcesAndTools=sources);deny('signed source schema',p=p,i=sign(p))
    for path in ('','/absolute','../escape','a/../b','./a','a/'+chr(0),'a/'+chr(127),'a/'+chr(159),'x'*4097,'曲'*1366,True,123):
        p=dict(proposal,sourcesAndTools=[dict(path=path,sha256='a'*64)])
        deny('signed invalid source path',p=p,i=sign(p))
    p=dict(proposal,sourcesAndTools=[dict(path=chr(0xd800),sha256='a'*64)])
    deny('invalid Unicode source path',p=p)
    p=dict(proposal,sourcesAndTools=[dict(path=f'{n}/'+('x'*300),sha256='a'*64) for n in range(3000)])
    deny('proposal byte bound',p=p,i=sign(p))
    print(f'owner execution intent audit: {accepted} accepted scopes, {rejected} rejected corruptions; no execution, presence, dispatch, source-byte or quality credit')
`], { env: { PATH: process.env.PATH }, encoding: 'utf8', timeout: 30000 });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /owner execution intent audit: .*no execution, presence, dispatch, source-byte or quality credit/);
  console.log(result.stdout.trim());
});

test('owner approval audit verifies only a frozen disabled snapshot with independent trust and time', () => {
  const { spawnSync } = require('node:child_process');
  const result = spawnSync('python3', ['-B', '-c', `
import sys,json,hashlib,copy,tempfile,pathlib,subprocess,os,uuid
from unittest.mock import patch
sys.path.insert(0,'scripts/auto-tag-eval-replay')
from importlib import import_module
m=import_module('audit-supervised')
assert callable(getattr(m,'owner_approval_audit',None)), 'missing standalone owner approval auditor'
canonical=lambda v:json.dumps(v,ensure_ascii=False,sort_keys=True,separators=(',',':')).encode()
sha=lambda b:hashlib.sha256(b).hexdigest()
env=dict(PATH=os.environ['PATH'],OPENSSL_CONF=os.devnull)
def openssl(*args):
    return subprocess.run(['openssl',*map(str,args)],env=env,check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=5).stdout
with tempfile.TemporaryDirectory(dir='.amp/in',prefix='owner-audit-') as temp:
    root=pathlib.Path(temp);private=root/'disposable-private.pem';public=root/'public.der'
    openssl('genpkey','-algorithm','ED25519','-out',private)
    openssl('pkey','-in',private,'-pubout','-outform','DER','-out',public)
    key=public.read_bytes()[-32:];trust=sha(key);now=1800000000
    policy='disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair'
    model='~deepseek/deepseek-v4-flash-latest'
    pricing=dict(promptPicoUsd=1,completionPicoUsd=2,catalogSha256='a'*64,termsSha256='b'*64)
    settings=dict(model=model,endpoint='https://openrouter.ai/api/v1',provider='openrouter',source='env_local_only')
    boundary=dict(endpoint='https://openrouter.ai/api/v1/chat/completions',settingsSha256=sha(json.dumps(settings,separators=(',',':')).encode()),
        pricingEvidence=pricing,textEstimatePicoUsd=10240,allowanceUsdCents=500,inputTokenIllustration=8192,outputTokens=1024,
        hardBillingCap=False,billingRiskAccepted=False,liveDispatchAvailable=False,readyForExecution=False,
        retention=dict(remote='documented_provider_defaults_for_public_metadata',local='until_result_review'))
    binding=dict(policy=policy,model=model,observedTarget='deepseek/deepseek-v4-flash-0731',providerClaim='Wafer',route='wafer/us',
        requestSha256=sha(b'prepared request'),verifierSha256=trust,liveAuthority=False,boundary=boundary)
    names=['disabled-crowd','disabled-billie','disabled-enya','assistance-billie','assistance-enya','assistance-crowd']
    report=dict(runId=str(uuid.uuid4()),state='frozen',membership=dict(requestedFieldSlots=216,public='盧廣仲 😀 \\u2028',
        nested=[None,True,False,-1,{'z':'line\\nquote"slash/','a':0}]),bindings=dict(executable=sha(b'executable'),modelRehearsal=binding),
        preparations={name:dict(preparationSha256=sha(name.encode()),requestSha256=binding['requestSha256'] if name=='assistance-crowd' else None) for name in names},
        sessions={},operations={},counters=dict(provider=0,inventory=0,model=0,reserved=0,arrived=0,completed=0),
        approvalSha256=None,approvalExpires=None,credentialLookups=0,testPolicy=policy,consumptions=0,denied=0,ingressDenials=[],
        ownerLive=True,mutationAdmissions=0,resultSha256=None,liveAuthority=False,recovery='read_only_incomplete_no_resume')
    for name in names:
        client=str(uuid.uuid4())
        report['sessions'][client]=dict(job=name,stage='preparation',endpoint='http://127.0.0.1:1234',manifest_sha256=sha(name.encode()),
            client=client,service=str(uuid.uuid4()),closed=True,finished=True,attached=True,sealed=True,native_live=False,fixture_live=False,observation_sha256=None)
    def freeze(r):
        r['checkpoint']=sha(canonical(dict(runId=r['runId'],membership=r['membership'],bindings=r['bindings'],preparations=r['preparations'],services=r['sessions'],counters=r['counters'])))
    def approve(r,signing_key=private,**claims):
        scope=dict(runId=r['runId'],checkpoint=r['checkpoint'],bindings=r['bindings'],membership=r['membership'],preparations=r['preparations'],policy=r['testPolicy'])
        payload=dict(version=1,runId=r['runId'],checkpoint=r['checkpoint'],scopeSha256=sha(canonical(scope)),nonce=str(uuid.uuid4()),expiresUnixSeconds=now+1200)
        payload.update(claims)
        (root/'payload').write_bytes(canonical(payload))
        openssl('pkeyutl','-sign','-rawin','-inkey',signing_key,'-in',root/'payload','-out',root/'signature')
        return dict(payload=payload,signature=list((root/'signature').read_bytes()))
    freeze(report);approval=approve(report)
    expected=dict(version=1,signatureVerified=True,verifierSha256=trust,approvalSha256=sha(canonical(approval)),
        checkpoint=report['checkpoint'],scopeSha256=approval['payload']['scopeSha256'],expiresUnixSeconds=now+1200,
        liveAuthority=False,userPresenceVerified=False,qualityCredit=0)
    before=copy.deepcopy((report,approval))
    real_run=subprocess.run;verification_directories=[]
    def observed_verification(args,**kwargs):
        assert args[:3]==['openssl','pkeyutl','-verify']
        assert kwargs['timeout']==5 and kwargs['stdin']==subprocess.DEVNULL
        assert kwargs['stdout']==kwargs['stderr']==subprocess.DEVNULL
        assert set(kwargs['env'])=={'PATH','OPENSSL_CONF'} and kwargs['env']['OPENSSL_CONF']==os.devnull
        directory=pathlib.Path(args[args.index('-inkey')+1]).parent;verification_directories.append(directory)
        assert set(p.name for p in directory.iterdir())=={'public.der','payload','signature'}
        assert (directory/'public.der').read_bytes()==bytes.fromhex('302a300506032b6570032100')+key
        assert (directory/'payload').read_bytes()==canonical(approval['payload'])
        assert (directory/'signature').read_bytes()==bytes(approval['signature'])
        return real_run(args,**kwargs)
    with patch.object(m.subprocess,'run',side_effect=observed_verification):
        assert m.owner_approval_audit(report,approval,key,trust,now)==expected
    assert len(verification_directories)==1 and not verification_directories[0].exists(), 'public verification temporary files leaked'
    assert (report,approval)==before, 'auditor mutated supplied evidence'
    accepted=1;rejected=0
    # Preparation profiles mutate disposable baselines before freezing; this
    # global bounded counter does not mean model dispatch or approved execution.
    for count in (208,4096):
        prepared=copy.deepcopy(report);prepared['mutationAdmissions']=count
        assert m.owner_approval_audit(prepared,approval,key,trust,now)==expected
        accepted+=1
    def deny(label,r=report,a=approval,k=key,t=trust,at=now):
        global rejected
        try: m.owner_approval_audit(r,a,k,t,at)
        except AssertionError: rejected+=1
        else: raise AssertionError('accepted '+label)
    import inspect
    optimized=dict(vars(m))
    exec(compile(inspect.getsource(m.owner_approval_audit),'optimized-owner-audit','exec',optimize=2),optimized)
    try: optimized['owner_approval_audit'](report,dict(approval,signature=[0]*64),key,trust,now)
    except AssertionError: rejected+=1
    else: raise AssertionError('optimized Python bypassed signature verification')
    for failure in (OSError('unavailable'),subprocess.TimeoutExpired('openssl',5)):
        with patch.object(m.subprocess,'run',side_effect=failure): deny('unavailable local verification')
    for seconds in (1,1800):
        a=approve(report,expiresUnixSeconds=now+seconds)
        assert m.owner_approval_audit(report,a,key,trust,now)['expiresUnixSeconds']==now+seconds
        accepted+=1
    for value in (now,now-1,now+1801,True,1.0,'future',-1,2**64):
        deny('expiry '+repr(value),a=approve(report,expiresUnixSeconds=value))
    for value in (True,-1,1.0,'1800000000',None,2**64): deny('time '+repr(value),at=value)
    for value in (None,[],b'',key[:-1],bytearray(key),list(key)): deny('public key type/length',k=value)
    for value in (None,True,'a'*64,trust.upper(),key): deny('independent digest',t=value)
    for hexkey,seed in (
        ('d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a',bytes.fromhex('9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60')),
        ('197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61',bytes([42])*32),
        ('4508a07aa941707f3eb2db94c8897a80b2c1197476b6de213ac273df7d86c4ff',bytes([43])*32)):
        (root/'fixture-seed.der').write_bytes(bytes.fromhex('302e020100300506032b657004220420')+seed)
        fixture_private=root/'public-fixture-private.pem'
        openssl('pkey','-inform','DER','-in',root/'fixture-seed.der','-out',fixture_private)
        assert openssl('pkey','-in',fixture_private,'-pubout','-outform','DER')[-32:].hex()==hexkey
        fixture=bytes.fromhex(hexkey);r=copy.deepcopy(report);r['bindings']['modelRehearsal']['verifierSha256']=sha(fixture)
        freeze(r);deny('validly signed public fixture approval',r=r,a=approve(r,signing_key=fixture_private),k=fixture,t=sha(fixture))
    for value in (None,[],{},dict(approval,extra=True)):
        deny('approval shape',a=value)
    for value in (None,'00',[],[0]*63,[0]*65,[True]*64,[-1]*64,[256]*64,[0.0]*64):
        deny('signature bytes',a=dict(approval,signature=value))
    for keyname,value in [('version',True),('version',2),('runId','other'),('checkpoint','f'*64),('scopeSha256','f'*64),('nonce',''),('nonce',True),('extra',1)]:
        deny('signed bad claim '+keyname,a=approve(report,**{keyname:value}))
    for keyname in approval['payload']:
        a=copy.deepcopy(approval);del a['payload'][keyname];deny('missing claim '+keyname,a=a)
    a=copy.deepcopy(approval);a['signature'][0]^=1;deny('signature corruption',a=a)
    a=copy.deepcopy(approval);a['payload']['nonce']=str(uuid.uuid4());deny('unsigned payload change',a=a)
    for field in ('membership','bindings','preparations','sessions','counters','checkpoint'):
        r=copy.deepcopy(report)
        if field=='checkpoint': r[field]='f'*64
        elif field=='counters': r[field]['provider']=1
        else: r[field]['injected']='changed'
        deny('checkpoint/scope drift '+field,r=r)
    semantic=[('state','executing'),('state','complete'),('consumptions',1),('consumptions',False),('credentialLookups',1),
        ('credentialLookups',False),('mutationAdmissions',False),('mutationAdmissions',-1),('mutationAdmissions',4097),('approvalSha256','a'*64),('approvalExpires',now+1),('resultSha256','b'*64),('liveAuthority',True),
        ('ownerLive',False),('ownerLive',1),('ownerLive',None)]
    for field,value in semantic:
        r=copy.deepcopy(report);r[field]=value;deny('report promotion '+field,r=r)
    binding_changes=[('policy','fixture'),('verifierSha256','a'*64),('liveAuthority',True),('offlineFixture',True),('offlineFixture',False),
        ('offlineHttp',True),('offlineResponse',{}),('boundary',{})]
    for field,value in binding_changes:
        r=copy.deepcopy(report);r['bindings']['modelRehearsal'][field]=value;freeze(r)
        deny('signed binding promotion '+field,r=r,a=approve(r))
    for field in boundary:
        r=copy.deepcopy(report);b=r['bindings']['modelRehearsal']['boundary']
        b[field]=True if type(b[field]) is bool else 501 if field=='allowanceUsdCents' else None
        freeze(r);deny('signed boundary drift '+field,r=r,a=approve(r))
    for field in ('hardBillingCap','billingRiskAccepted','liveDispatchAvailable','readyForExecution'):
        for value in (0,1,None,'false',[]):
            r=copy.deepcopy(report);r['bindings']['modelRehearsal']['boundary'][field]=value
            freeze(r);deny('disabled flag must be exactly false '+field,r=r,a=approve(r))
    for field in ('closed','finished','attached','sealed','native_live','fixture_live','stage','job'):
        r=copy.deepcopy(report);s=next(iter(r['sessions'].values()));s[field]=not s[field] if type(s[field]) is bool else 'invalid'
        freeze(r);deny('signed invalid session '+field,r=r,a=approve(r))
    for field in ('endpoint','service'):
        for value in (None,True,[],{},1,''):
            r=copy.deepcopy(report);next(iter(r['sessions'].values()))[field]=value
            freeze(r);deny('signed malformed session '+field,r=r,a=approve(r))
    for field in ('sessions','preparations'):
        r=copy.deepcopy(report);r[field].pop(next(iter(r[field])));freeze(r);deny('missing preparation '+field,r=r,a=approve(r))
    r=copy.deepcopy(report);sid=next(iter(r['sessions']))
    r['operations']['one']=dict(session=sid,category='model',request_sha256='a'*64,dispatched=True,arrived=True,completed=True)
    r['counters'].update(model=1,reserved=1,arrived=1,completed=1);freeze(r);deny('signed model operation',r=r,a=approve(r))
    r['operations']['one']['category']='provider';r['counters'].update(model=0,provider=1)
    freeze(r);a=approve(r);assert m.owner_approval_audit(r,a,key,trust,now)['signatureVerified'] is True;accepted+=1
    r['operations']['one']['completed']=False;r['counters']['completed']=0;freeze(r);deny('pending operation',r=r,a=approve(r))
    for value in (0.0,float('nan'),float('inf'),2**64,-2**63-1):
        r=copy.deepcopy(report);r['membership']['invalid']=value;freeze(r);deny('ambiguous scope number',r=r,a=approve(r))
    for value in (None,[],True): deny('report type',r=value)
    r=copy.deepcopy(report);r['bindings']['modelRehearsalPlan']=dict(nativePrepared=False)
    freeze(r);deny('fixture plan',r=r,a=approve(r))
    plan=dict(nativePrepared=True,model=model,observedTarget=binding['observedTarget'],route=binding['route'],provider=binding['providerClaim'],
        input=dict(artist='盧廣仲',album=None,tracks=['One','Other']))
    r=copy.deepcopy(report);r['bindings']['modelRehearsalPlan']=plan;freeze(r)
    assert m.owner_approval_audit(r,approve(r),key,trust,now)['signatureVerified'] is True;accepted+=1
    for value in (None,[],{},dict(artist=True,album=None,tracks=[]),dict(artist=None,album=None,tracks=[False]),dict(artist=None,album=None,tracks='one')):
        r=copy.deepcopy(report);r['bindings']['modelRehearsalPlan']=dict(plan,input=value)
        freeze(r);deny('signed malformed native input',r=r,a=approve(r))
    r=copy.deepcopy(report);r['membership']['oversized']='x'*2097153;freeze(r);deny('unbounded scope',r=r,a=approve(r))
    assert callable(getattr(m,'owner_offline_completed_audit',None)), 'missing explicit completed owner-offline auditor'
    with tempfile.TemporaryDirectory(dir='.amp/in/artifacts',prefix='owner-completed-unit-') as temp_run:
        run=pathlib.Path(temp_run).resolve()/'run';run.mkdir()
        put=lambda name,value:(run/name).write_text(json.dumps(value,ensure_ascii=False))
        (root/'native-evaluator').write_bytes(b'disposable stand-in executable bytes')
        (root/'native-evaluator').chmod(0o700)
        (run.parent/'public-native-plan.json').write_bytes(canonical(plan))
        (run.parent/'public-fake-input').mkdir()
        credential=run.parent/'public-fake-input/.env.local'
        credential.write_text('LLM_API_KEY=MOCK_ONLY_SECRET_CANARY\\nLLM_MODEL='+model+'\\n');credential.chmod(0o600)
        r=copy.deepcopy(report);r['membership']=m.load(m.ROOT/'test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json')
        r['bindings'].update(modelRehearsalPlan=plan,disposableRunRoot=str(run),executable=sha((root/'native-evaluator').read_bytes()))
        put('native-supervision.json',dict(generation=str(uuid.uuid4())))
        relative=lambda p:str(p.resolve().relative_to(m.ROOT))
        locks=[dict(path=relative(p),sha256=sha(p.read_bytes())) for p in (root/'native-evaluator',run.parent/'public-native-plan.json',credential,run/'native-supervision.json')]
        r['bindings']['sourcesAndTools']=locks
        proposal=dict(version=1,proposalId=str(uuid.uuid4()),executionKind='offlineRehearsal',executableSha256=r['bindings']['executable'],
            membershipSha256=sha(canonical(r['membership'])),nativePlanSha256=sha(canonical(plan)),requestSha256=binding['requestSha256'],
            verifierSha256=trust,boundaryPricingSha256=sha(canonical(boundary)),contextSha256=sha(canonical(binding)),
            sourcesAndTools=locks[:-1]+[dict(path=locks[-1]['path'],runtime='nativeSupervision')])
        claims=dict(version=1,proposalId=proposal['proposalId'],proposalSha256=sha(canonical(proposal)),executionKind='offlineRehearsal',
            nonce=str(uuid.uuid4()),issuedUnixSeconds=now-600,expiresUnixSeconds=now+1200,hostedEstimateRiskAccepted=False)
        (root/'payload').write_bytes(b'soundrobe/owner-execution-intent/v1'+bytes([0])+canonical(claims))
        openssl('pkeyutl','-sign','-rawin','-inkey',private,'-in',root/'payload','-out',root/'signature')
        intent=dict(claims,signature=list((root/'signature').read_bytes()))
        owner={key:intent[key] for key in ('version','proposalId','proposalSha256','executionKind','nonce','expiresUnixSeconds')}
        owner['intentSha256']=sha(canonical(intent));r['bindings']['ownerExecution']=owner
        freeze(r);a=approve(r,nonce=intent['nonce'],expiresUnixSeconds=intent['expiresUnixSeconds'])
        packet=dict(version=1,proposal=proposal,intent=intent,ownerExecution=owner,offlineCredential=locks[2])
        put('owner-execution-bootstrap.json',packet);put('frozen-checkpoint.json',r);put('requested-membership.json',r['membership'])
        coordination={key:r[key] for key in ('runId','checkpoint','bindings','membership','preparations','counters','credentialLookups')}
        coordination.update(componentCoordinationOnly=True,liveAuthority=False,humanPresenceVerified=False,qualityCertificate=False,successCertificate=False,
            approvalSha256=sha(canonical(a)),approvalExpires=a['payload']['expiresUnixSeconds'],consumptions=1,state='executing',actualFakeLookups=0)
        put('owner-approval-coordination.json',coordination)
        put('results.json',dict(mode='mock_only',executions=[]))
        final=copy.deepcopy(r);final.update(state='provisional_complete',ownerLive=False,consumptions=1,approvalSha256=sha(canonical(a)),
            approvalExpires=a['payload']['expiresUnixSeconds'],credentialLookups=1,actualFakeLookups=1,resultSha256=sha((run/'results.json').read_bytes()))
        (run/'authority').mkdir();put('authority/provisional.json',final);put('authority-provisional.json',final)
        def completed(**kwargs):
            return m.owner_offline_completed_audit(run,proposal=kwargs.get('proposal',proposal),intent=kwargs.get('intent',intent),
                approval=kwargs.get('approval',a),public_key=kwargs.get('public_key',key),expected_verifier_sha256=kwargs.get('pin',trust),
                verified_at=kwargs.get('verified_at',now), trusted_executable=root/'native-evaluator')
        # Unit-test the signature/scope join separately from downstream native
        # completion; the stub is explicitly NOT a completed rehearsal proof.
        with patch.object(m,'audit',return_value=dict(downstream='unit-stub')) as downstream:
            checked=completed();assert checked['downstream']=='unit-stub'
            assert checked['ownerExecution']['sourceBytesVerified'] is True
            assert checked['ownerExecution']['userPresenceVerified'] is False
            assert downstream.call_count==1 and downstream.call_args.kwargs==dict(_owner_public_key=key)
        accepted_substitutions=[]
        for field,changes in [('nonce',dict(nonce=str(uuid.uuid4()))),
                ('expiry',dict(expiresUnixSeconds=intent['expiresUnixSeconds']+300))]:
            replacement_claims=dict(nonce=intent['nonce'],expiresUnixSeconds=intent['expiresUnixSeconds'])
            replacement_claims.update(changes)
            replacement=approve(r,**replacement_claims)
            assert m.owner_approval_audit(r,replacement,key,trust,now)['signatureVerified'] is True
            # Keep downstream receipts coherent so only the signed intent join
            # can reject this otherwise valid, independently signed approval.
            replacement_binding=dict(approvalSha256=sha(canonical(replacement)),
                approvalExpires=replacement['payload']['expiresUnixSeconds'])
            put('owner-approval-coordination.json',dict(coordination,**replacement_binding))
            put('authority/provisional.json',dict(final,**replacement_binding))
            put('authority-provisional.json',dict(final,**replacement_binding))
            with patch.object(m,'audit',return_value={}) as downstream:
                try: completed(approval=replacement)
                except AssertionError as error:
                    assert str(error)=='owner approval '+field+' differs from signed intent', str(error)
                    assert downstream.call_count==0, 'invalid owner join reached completion audit'
                else: accepted_substitutions.append(field)
        put('owner-approval-coordination.json',coordination)
        put('authority/provisional.json',final);put('authority-provisional.json',final)
        assert not accepted_substitutions, 'accepted independently signed owner approval substitutions: '+str(accepted_substitutions)
        print('owner completed join: matching issuance accepted; independently signed nonce substitution and extended expiry rejected before downstream completion')
        try: completed()
        except (AssertionError,FileNotFoundError,KeyError): pass
        else: raise AssertionError('prepared synthetic scope promoted without completion evidence')
        def reject_completed(label,**kwargs):
            with patch.object(m,'audit',return_value={}):
                try: completed(**kwargs)
                except (AssertionError,ValueError,KeyError): pass
                else: raise AssertionError('accepted owner completed corruption: '+label)
        reject_completed('wrong pin',pin='0'*64)
        reject_completed('expired',verified_at=now+1200)
        reject_completed('unsigned proposal',proposal=dict(proposal,requestSha256='0'*64))
        reject_completed('corrupt approval',approval=dict(a,signature=[0]*64))
        for name,record,changes in [('owner-execution-bootstrap.json',packet,dict(ownerExecution=dict(owner,executionKind='hostedSingleRequest'))),
            ('owner-approval-coordination.json',coordination,dict(consumptions=0)),
            ('owner-approval-coordination.json',coordination,dict(approvalSha256='0'*64)),
            ('authority/provisional.json',final,dict(state='frozen')),
            ('authority/provisional.json',final,dict(resultSha256='0'*64)),
            ('authority/provisional.json',final,dict(bindings=dict(r['bindings'],ownerExecution=dict(owner,nonce=str(uuid.uuid4())))))]:
            put(name,dict(record,**changes));reject_completed(name);put(name,record)
        (root/'native-evaluator').write_bytes(b'drifted');reject_completed('actual executable source bytes')
        (root/'native-evaluator').write_bytes(b'disposable stand-in executable bytes')
        # Re-sign real disposable scopes so each response-file rejection must
        # come from public source validation, not an invalid signature/hash of
        # the proposal. The downstream stub still makes no completion claim.
        original_r,original_proposal,original_intent=copy.deepcopy((r,proposal,intent))
        raw=canonical(dict(model=plan['observedTarget'],provider='Wafer',usage=dict(prompt_tokens=23,completion_tokens=11,total_tokens=34),
            choices=[dict(finish_reason='stop',message=dict(content=canonical(dict(genre='Jazz',confidence=0.8)).decode()))]))
        private_cases={'private-key-source':'private-review-marker.pk8','private-json-source':'settings.json',
            'hidden-source':'.private.rs','fake-credential-lookalike':'.env.local',
            'private-executable-slot':'private-executable.pk8'}
        for case in ('valid','boundary','changed','missing','oversize','hardlink','symlink','alias','wrong-plan-name','source-order',*private_cases):
            case_root=run.parent/case;case_root.mkdir()
            public_plan=case_root/('other-plan.json' if case=='wrong-plan-name' else 'public-native-plan.json')
            public_plan.write_bytes(canonical(plan));response=case_root/'response-fixture.json'
            response.write_bytes(raw+b' '*((65537 if case=='oversize' else 65536)-len(raw)) if case in ('oversize','boundary') else raw)
            response_lock=dict(path=relative(response),sha256=sha(response.read_bytes()))
            r,proposal,intent=copy.deepcopy((original_r,original_proposal,original_intent))
            locks=copy.deepcopy(r['bindings']['sourcesAndTools'])
            locks[1]=dict(path=relative(public_plan),sha256=sha(public_plan.read_bytes()))
            if case=='alias': response_lock['path']=str(case_root.relative_to(m.ROOT))+'/./response-fixture.json'
            locks.insert(-1,response_lock)
            private_source=None
            if case in private_cases:
                private_source=case_root/private_cases[case]
                private_source.write_bytes(b'BENIGN DISPOSABLE MARKER: NOT A SECRET OR KEY')
                private_source.chmod(0o700)
                private_lock=dict(path=relative(private_source),sha256=sha(private_source.read_bytes()))
                if case=='private-executable-slot':
                    locks[0]=private_lock
                    r['bindings']['executable']=proposal['executableSha256']=private_lock['sha256']
                else: locks.insert(-1,private_lock)
            r['bindings']['sourcesAndTools']=locks
            proposal['sourcesAndTools']=locks[:-1]+[dict(path=locks[-1]['path'],runtime='nativeSupervision')]
            claims={k:v for k,v in intent.items() if k!='signature'};claims['proposalSha256']=sha(canonical(proposal))
            (root/'payload').write_bytes(b'soundrobe/owner-execution-intent/v1'+bytes([0])+canonical(claims))
            openssl('pkeyutl','-sign','-rawin','-inkey',private,'-in',root/'payload','-out',root/'signature')
            intent=dict(claims,signature=list((root/'signature').read_bytes()))
            owner={k:intent[k] for k in ('version','proposalId','proposalSha256','executionKind','nonce','expiresUnixSeconds')}
            owner['intentSha256']=sha(canonical(intent));r['bindings']['ownerExecution']=owner
            if case=='source-order': r['bindings']['sourcesAndTools']=list(reversed(locks))
            freeze(r);a=approve(r,nonce=intent['nonce'],expiresUnixSeconds=intent['expiresUnixSeconds'])
            packet=dict(version=1,proposal=proposal,intent=intent,ownerExecution=owner,offlineCredential=locks[2])
            put('owner-execution-bootstrap.json',packet);put('frozen-checkpoint.json',r)
            coordination={k:r[k] for k in ('runId','checkpoint','bindings','membership','preparations','counters','credentialLookups')}
            coordination.update(componentCoordinationOnly=True,liveAuthority=False,humanPresenceVerified=False,qualityCertificate=False,successCertificate=False,
                approvalSha256=sha(canonical(a)),approvalExpires=a['payload']['expiresUnixSeconds'],consumptions=1,state='executing',actualFakeLookups=0)
            put('owner-approval-coordination.json',coordination)
            final=copy.deepcopy(r);final.update(state='provisional_complete',ownerLive=False,consumptions=1,approvalSha256=sha(canonical(a)),
                approvalExpires=a['payload']['expiresUnixSeconds'],credentialLookups=1,actualFakeLookups=1,resultSha256=sha((run/'results.json').read_bytes()))
            put('authority/provisional.json',final)
            if case=='changed': response.write_bytes(raw+b' ')
            if case=='missing': response.unlink()
            if case in ('hardlink','symlink'):
                target_file=case_root/'same-response-bytes';response.rename(target_file)
                os.link(target_file,response) if case=='hardlink' else response.symlink_to(target_file)
            source_opens=[];actual_open=pathlib.Path.open
            def observed_open(file,*args,**kwargs):
                if file==private_source: source_opens.append(str(file))
                return actual_open(file,*args,**kwargs)
            with patch.object(pathlib.Path,'open',new=observed_open), patch.object(m,'audit',return_value=dict(downstream='unit-stub')) as downstream:
                if case in ('valid','boundary'):
                    assert completed()['downstream']=='unit-stub';assert downstream.call_count==1
                else:
                    try: completed()
                    except (AssertionError,ValueError,KeyError,OSError): assert downstream.call_count==0
                    else: raise AssertionError('accepted signed response source corruption: '+case)
            assert not source_opens, 'opened nonpublic signed source before rejection: '+case
        print('owner response source join: valid and 65536-byte signed scopes accepted; thirteen signed corruptions rejected before downstream completion and nonpublic source reads')
        print('owner completed API: real disposable signatures and source/scope joins; downstream stub explicitly not completion proof')
    print(f'owner approval audit: {accepted} accepted snapshots, {rejected} rejected corruptions; no execution, presence or quality credit')
`], { env: { PATH: process.env.PATH }, encoding: 'utf8', timeout: 30000 });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /owner approval audit: .*no execution, presence or quality credit/);
  console.log(result.stdout.trim());
});

test('HTTP response audit independently derives bounded claims without authority or diagnostic leakage', () => {
  const { spawnSync } = require('node:child_process');
  const result = spawnSync('python3', ['-B', '-c', `
import sys,json,hashlib,copy
sys.path.insert(0,'scripts/auto-tag-eval-replay')
from importlib import import_module
m=import_module('audit-supervised')
assert callable(getattr(m,'http_response_audit',None)), 'missing independent HTTP response auditor'
sha=lambda b:hashlib.sha256(b).hexdigest()
encode=lambda v:json.dumps(v,ensure_ascii=False,separators=(',',':')).encode()
request=b'{"model":"request-alias"}'
model='response-model';provider='claimed-provider'
answer=dict(genre='Jazz',confidence=0.8)
body=dict(model=model,provider=provider,choices=[dict(finish_reason='stop',message=dict(content=encode(answer).decode()))],
    usage=dict(prompt_tokens=7,completion_tokens=3,total_tokens=10),diagnostic='RAW_SECRET_CANARY')
projection=dict(model=model,provider=provider,genre='Jazz',confidence=0.8,
    usage=dict(promptTokens=7,completionTokens=3,totalTokens=10,reasoningTokens=0))
count=0
def check(label,change=lambda b:None,validation='accepted',project=None,status=200,raw=None,req=request,transport='offline_local_http',damage=None,error=None):
    global count
    value=copy.deepcopy(body);change(value)
    response=encode(value) if raw is None else raw
    expected=dict(version=1,transport=transport,requestSha256=sha(req),validation=validation,
        projection=None if validation=='rejected' else copy.deepcopy(projection if project is None else project))
    receipt=dict(sha256=sha(response),bytes=len(response),status=status,identityClaimsVerified=False,
        pricingVerified=False,usageClaimsVerified=False,observation='native_http_response',remoteArrivalVerified=False,http=copy.deepcopy(expected))
    if damage: damage(receipt)
    try:
        observed=m.http_response_audit(req,response,receipt,model,provider,transport)
    except AssertionError as exc:
        assert error is not None and error in str(exc), (label,'wrong rejection',str(exc),error)
    else:
        assert error is None, (label,'accepted corrupt receipt')
        assert observed==expected, (label,observed,expected)
        assert observed is not receipt['http']
        if observed['projection'] is not None:
            assert observed['projection'] is not receipt['http']['projection']
        assert 'RAW_SECRET_CANARY' not in json.dumps(observed)
        assert set(observed)=={'version','transport','requestSha256','validation','projection'}
    count+=1
def content(value):
    return lambda b:b['choices'][0]['message'].update(content=encode(value).decode())
cases=[
    ('accepted',{}),('hosted HTTPS claims still unverified',dict(transport='hosted_https')),
    ('confidence numeric equality',dict(change=content(dict(genre='Jazz',confidence=1.0)),project=dict(projection,confidence=1))),
    ('threshold accepted',dict(change=content(dict(genre='Jazz',confidence=0.6)),project=dict(projection,confidence=0.6))),
    ('low confidence withheld',dict(change=content(dict(genre='Jazz',confidence=0.59)),validation='withheld',project=dict(projection,genre='',confidence=0.59))),
    ('empty genre withheld',dict(change=content(dict(genre='',confidence=0.8)),validation='withheld',project=dict(projection,genre=''))),
    ('optional empty fields',dict(change=lambda b:b['choices'][0]['message'].update(refusal=None,function_call=None,tool_calls=[],reasoning='',reasoning_content=None))),
    ('usage limit',dict(change=lambda b:b.update(usage=dict(prompt_tokens=8192,completion_tokens=1024,total_tokens=9216,completion_tokens_details=dict(reasoning_tokens=0))),project=dict(projection,usage=dict(promptTokens=8192,completionTokens=1024,totalTokens=9216,reasoningTokens=0)))),
    ('absent reasoning tokens',dict(change=lambda b:b['usage'].update(completion_tokens_details={}))),
    ('genre UTF8 boundary',dict(change=content(dict(genre='é'*128,confidence=0.8)),project=dict(projection,genre='é'*128))),
    ('genre UTF8 overflow',dict(change=content(dict(genre='é'*128+'x',confidence=0.8)),validation='rejected')),
    ('request byte boundary',dict(req=b'x'*32768)),
    ('request byte overflow',dict(req=b'x'*32769,error='request bytes')),
    ('invalid UTF8',dict(raw=b'\\xff',validation='rejected')),
    ('malformed JSON',dict(raw=b'{',validation='rejected')),
    ('non-object JSON',dict(raw=b'[]',validation='rejected')),
    ('nonfinite confidence',dict(change=content(dict(genre='Jazz',confidence=float('nan'))),validation='rejected')),
    ('invalid JSON diagnostic constant',dict(change=lambda b:b.update(diagnostic=float('nan')),validation='rejected')),
    ('duplicate genre field',dict(change=lambda b:b['choices'][0]['message'].update(content='{"genre":"Jazz","genre":"Jazz","confidence":0.8}'),validation='rejected')),
    ('duplicate confidence field',dict(change=lambda b:b['choices'][0]['message'].update(content='{"genre":"Jazz","confidence":0.8,"confidence":0.8}'),validation='rejected')),
    ('overflowing diagnostic float',dict(raw=encode(body).replace(b'"diagnostic":"RAW_SECRET_CANARY"',b'"diagnostic":1e400'),validation='rejected')),
    ('overflowing diagnostic integer',dict(raw=encode(body).replace(b'"diagnostic":"RAW_SECRET_CANARY"',b'"diagnostic":'+b'9'*400),validation='rejected')),
    ('negative-zero usage is not unsigned integer evidence',dict(raw=encode(dict(body,usage=dict(prompt_tokens=0,completion_tokens=3,total_tokens=3))).replace(b'"prompt_tokens":0',b'"prompt_tokens":-0'),validation='rejected')),
    ('infinite confidence',dict(change=content(dict(genre='Jazz',confidence=float('inf'))),validation='rejected')),
]
for status in (100,201,400,599): cases.append(('status '+str(status),dict(status=status,validation='rejected')))
for key,value in [('genre',True),('genre','bad\\n'),('genre','bad\\x7f'),('genre','bad\\x85'),('confidence',True),('confidence',-0.1),('confidence',1.1),('confidence','0.8')]:
    cases.append(('answer '+key+repr(value),dict(change=content(dict(answer,**{key:value})),validation='rejected')))
for value in ({'genre':'Jazz'},dict(answer,extra=1),['Jazz'],None):
    cases.append(('answer shape '+repr(value),dict(change=content(value),validation='rejected')))
for key,value in [('refusal','no'),('refusal',False),('function_call',{}),('tool_calls',[{}]),('tool_calls',False),('reasoning','secret'),('reasoning_content','secret'),('reasoning',False),('content',{}),('content','not JSON')]:
    cases.append(('message '+key+repr(value),dict(change=lambda b,k=key,v=value:b['choices'][0]['message'].update({k:v}),validation='rejected')))
for key,value in [('model','wrong'),('provider','wrong'),('model',True),('provider',None),('choices',[]),('choices',[{},{}]),('choices',{}),('usage',None)]:
    cases.append(('outer '+key+repr(value),dict(change=lambda b,k=key,v=value:b.update({k:v}),validation='rejected')))
cases.extend([('unfinished',dict(change=lambda b:b['choices'][0].update(finish_reason='length'),validation='rejected')),
    ('missing message',dict(change=lambda b:b['choices'][0].pop('message'),validation='rejected')),
    ('missing finish reason',dict(change=lambda b:b['choices'][0].pop('finish_reason'),validation='rejected')),
    ('missing content',dict(change=lambda b:b['choices'][0]['message'].pop('content'),validation='rejected'))])
for key in ('model','provider','choices','usage'):
    cases.append(('missing response '+key,dict(change=lambda b,k=key:b.pop(k),validation='rejected')))
for key,value in [('prompt_tokens',True),('prompt_tokens',7.0),('prompt_tokens',8193),('prompt_tokens',-1),('completion_tokens',1025),('total_tokens',11),('total_tokens',True),('completion_tokens_details',None),('completion_tokens_details',[]),('completion_tokens_details',dict(reasoning_tokens=1)),('completion_tokens_details',dict(reasoning_tokens=False))]:
    cases.append(('usage '+key+repr(value),dict(change=lambda b,k=key,v=value:b['usage'].update({k:v}),validation='rejected')))
for key in ('prompt_tokens','completion_tokens','total_tokens'):
    cases.append(('missing usage '+key,dict(change=lambda b,k=key:b['usage'].pop(k),validation='rejected')))
# Limits count bytes, including JSON whitespace, rather than Unicode code points.
inner=encode(dict(answer,genre='é')).decode()
for size in (4096,4097):
    cases.append(('content byte '+str(size),dict(change=lambda b,n=size:b['choices'][0]['message'].update(content=inner+' '*(n-len(inner.encode()))),project=dict(projection,genre='é'),validation='accepted' if size==4096 else 'rejected')))
utf8=encode(dict(body,diagnostic='é'))
for size in (65536,65537):
    cases.append(('response byte '+str(size),dict(raw=utf8+b' '*(size-len(utf8)),**({} if size==65536 else {'error':'response bytes'}))))
for label,options in cases: check(label,**options)
damage=[('response hash',lambda r:r.update(sha256='0'*64),'response binding'),
    ('response length',lambda r:r.update(bytes=r['bytes']+1),'response binding'),
    ('boolean bytes',lambda r:r.update(bytes=True),'response binding'),
    ('request hash',lambda r:r['http'].update(requestSha256='0'*64),'http receipt'),
    ('changed projection',lambda r:r['http']['projection'].update(genre='Invented'),'http receipt'),
    ('boolean confidence',lambda r:r['http']['projection'].update(confidence=True),'http receipt'),
    ('boolean version',lambda r:r['http'].update(version=True),'http receipt'),
    ('float version',lambda r:r['http'].update(version=1.0),'http receipt'),
    ('boolean usage',lambda r:r['http']['projection']['usage'].update(reasoningTokens=False),'http receipt'),
    ('float usage',lambda r:r['http']['projection']['usage'].update(promptTokens=7.0),'http receipt'),
    ('HTTP extra diagnostic',lambda r:r['http'].update(diagnostic='RAW_SECRET_CANARY'),'http receipt'),
    ('receipt extra header',lambda r:r.update(headers={'authorization':'secret'}),'receipt fields'),
    ('receipt not object',lambda r:r.clear(),'receipt fields'),
    ('old observation',lambda r:r.update(observation='offline_injected_native_response'),'observation'),
    ('transport drift',lambda r:r['http'].update(transport='hosted_https'),'http receipt'),
    ('projection extra authority',lambda r:r['http']['projection'].update(liveAuthority=True),'http receipt'),
    ('null projection accepted',lambda r:r['http'].update(projection=None),'http receipt'),
    ('forged rejected',lambda r:r['http'].update(validation='rejected',projection=None),'http receipt')]
for key in ('identityClaimsVerified','pricingVerified','usageClaimsVerified','remoteArrivalVerified'):
    for value in (True,0,None): damage.append((key+repr(value),lambda r,k=key,v=value:r.update({k:v}),'verification flags'))
for value in (True,200.0,99,600,'200'):
    damage.append(('invalid status '+repr(value),lambda r,v=value:r.update(status=v),'status'))
for key in ('sha256','bytes','status','identityClaimsVerified','pricingVerified','usageClaimsVerified','observation','remoteArrivalVerified','http'):
    damage.append(('missing receipt '+key,lambda r,k=key:r.pop(k),'receipt fields'))
for key in ('version','transport','requestSha256','validation','projection'):
    damage.append(('missing http '+key,lambda r,k=key:r['http'].pop(k),'http receipt'))
for key in ('model','provider','genre','confidence','usage'):
    damage.append(('missing projection '+key,lambda r,k=key:r['http']['projection'].pop(k),'http receipt'))
for key in ('promptTokens','completionTokens','totalTokens','reasoningTokens'):
    damage.append(('missing projected usage '+key,lambda r,k=key:r['http']['projection']['usage'].pop(k),'http receipt'))
for label,mutate,error in damage: check(label,damage=mutate,error=error)
check('unknown transport',transport='fixture',error='transport')
check('boolean confidence cannot equal one',change=content(dict(answer,confidence=1)),project=dict(projection,confidence=1),damage=lambda r:r['http']['projection'].update(confidence=True),error='http receipt')
check('rejected cannot carry projection',status=400,validation='rejected',damage=lambda r:r['http'].update(projection=projection),error='http receipt')
check('withheld cannot expose genre',change=content(dict(genre='Jazz',confidence=0.5)),validation='withheld',project=dict(projection,genre='',confidence=0.5),damage=lambda r:r['http']['projection'].update(genre='Jazz'),error='http receipt')
check('corrupt exact request bytes',damage=lambda r:r['http'].update(requestSha256=sha(request+b' ')),error='http receipt')
print(str(count)+' independent HTTP audit cases passed; no authority or quality promotion')
`], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /independent HTTP audit cases passed; no authority or quality promotion/);
  console.log(result.stdout.trim());
});

const receipt = () => ({ state: 'provisional_complete', consumptions: 1, denied: 0,
  ownerLive: false, liveAuthority: false, runId: 'fresh-run', checkpoint: 'frozen',
  membership: { requestedFieldSlots: 216 }, bindings: { supervision: { generation: 'fresh-generation' } },
  counters: { reserved: 1, arrived: 1, completed: 1 }, preparations: {},
  resultSha256: 'a'.repeat(64),
  sessions: Object.fromEntries(Array.from({ length: 12 }, (_, i) => [String(i), { closed: true }])),
  operations: { one: { dispatched: true, arrived: true, completed: true } } });

test('explicitly released staging is valid evidence, never complete authority', () => {
  const value = receipt();
  assert.doesNotThrow(() => validateProvisionalReceipt(value, structuredClone(value)));
  for (const mutate of [r => r.state = 'complete', r => r.ownerLive = true,
    r => r.membership.requestedFieldSlots--, r => r.resultSha256 = 'b'.repeat(64),
    r => r.operations.one.completed = false, r => r.bindings.supervision.generation = 'other']) {
    const changed = structuredClone(value); mutate(changed);
    assert.throws(() => validateProvisionalReceipt(changed, value));
  }
});

for (const mode of ['fixture', 'offline hosted', 'offline HTTP', 'owner offline', 'owner response Jazz', 'owner response Soul', 'owner response withheld', 'owner response rejected', 'response Jazz', 'response Soul', 'response withheld', 'response rejected', 'response status']) test(`prepared ${mode} model rehearsal audit joins exact alias request, response receipts and retained provenance`, () => {
  const owner = mode.startsWith('owner ');
  const custom = mode.includes('response '), withheld = mode.endsWith('response withheld');
  const rejected = mode.endsWith('response rejected') || mode === 'response status';
  const hosted = mode !== 'fixture', offlineHttp = mode === 'offline HTTP' || custom || owner;
  const status = mode === 'response status' ? 429 : 200;
  const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
  const { spawnSync } = require('node:child_process');
  const root = fs.mkdtempSync(path.resolve(owner ? '.amp/in/artifacts/model-audit-' : '.amp/in/model-audit-'));
  const sha = b => crypto.createHash('sha256').update(b).digest('hex');
  const canonical = value => JSON.stringify(value, (_, v) => v && typeof v === 'object' && !Array.isArray(v)
    ? Object.fromEntries(Object.keys(v).sort().map(k => [k, v[k]])) : v);
  const plan = { model: '~deepseek/deepseek-v4-flash-latest', observedTarget: 'deepseek/deepseek-v4-flash-0731',
    route: 'wafer/us', provider: 'Wafer', input: { artist: '盧廣仲', album: 'Album', tracks: ['Intro', 'Title'] } };
  if (hosted) plan.offlineHosted = { promptPicoUsd: 1000, completionPicoUsd: 2000,
    catalogSha256: 'a'.repeat(64), termsSha256: 'b'.repeat(64) };
  if (offlineHttp) plan.offlineHttp = true;
  const policy = hosted ? 'disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair'
    : 'offline-model-rehearsal-v1;separate-verifier;fake-credential;loopback-only;no-live;no-retry;no-repair';
  const body = { model: plan.model, temperature: 0.2, max_tokens: 1024, reasoning: { enabled: false }, stream: false,
    provider: { allow_fallbacks: false, require_parameters: true, only: [plan.route], order: [plan.route] },
    messages: [{ role: 'system', content: 'Infer a conservative music genre from the supplied artist, album, and track titles. Return a concise Discogs-style comma-separated genre and confidence. Use low confidence when the evidence is insufficient.' },
      { role: 'user', content: canonical(plan.input) }],
    response_format: { type: 'json_schema', json_schema: { name: 'GenreFillResponse', schema: {
      type: 'object', properties: { genre: { type: 'string' }, confidence: { type: 'number' } }, required: ['genre', 'confidence'] } } } };
  const request = Buffer.from(canonical(body)), requestSha256 = sha(request);
  const responseValue = { genre: custom ? (mode.endsWith('response Soul') ? 'Soul' : 'Jazz') : 'MOCK_ACCEPTED_GENRE_CANARY', confidence: withheld ? 0.59 : mode.endsWith('response Soul') ? 0.91 : 0.8 };
  const usage = mode.endsWith('response Soul') ? { prompt_tokens: 31, completion_tokens: 5, total_tokens: 36 }
    : custom ? { prompt_tokens: 23, completion_tokens: 11, total_tokens: 34 }
    : { prompt_tokens: 7, completion_tokens: 3, total_tokens: 10 };
  const response = mode.endsWith('response rejected') ? '{' : (hosted ? canonical : JSON.stringify)({ model: plan.observedTarget, provider: plan.provider,
    choices: [{ finish_reason: 'stop', message: { content: (hosted ? canonical : JSON.stringify)(responseValue) } }],
    usage, diagnostic: 'MOCK_RAW_DIAGNOSTIC_CANARY' });
  if (custom && !owner) plan.offlineResponse = { sha256: sha(response), status };
  const names = ['disabled-billie', 'disabled-enya', 'disabled-crowd', 'assistance-billie', 'assistance-enya', 'assistance-crowd'];
  const files = {};
  const put = (file, value) => files[file] = value;
  // Public credential fixture sits beside the plan, outside the output tree.
  const output = path.join(root, 'output');
  const planFile = path.join(root, owner && custom ? 'public-native-plan.json' : 'public-plan.json');
  const responseFile = path.join(root, 'response-fixture.json');
  fs.writeFileSync(planFile, JSON.stringify(plan));
  const credentialFile = path.join(root, 'credential-fixture/.env.local');
  const credential = `LLM_API_KEY=MOCK_ONLY_SECRET_CANARY\nLLM_MODEL=${plan.model}\n`;
  if (hosted) {
    fs.mkdirSync(path.dirname(credentialFile));
    fs.writeFileSync(credentialFile, credential, { mode: 0o600 });
  }
  put('selection.json', { model: plan.model, modelRehearsalPlan: structuredClone(plan) });
  const binding = { policy, model: plan.model, observedTarget: plan.observedTarget, providerClaim: plan.provider,
    route: plan.route, requestSha256, verifierSha256: sha(Buffer.from(hosted
      ? '4508a07aa941707f3eb2db94c8897a80b2c1197476b6de213ac273df7d86c4ff'
      : '197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61', 'hex')), liveAuthority: false };
  if (hosted) binding.offlineFixture = true;
  if (offlineHttp) binding.offlineHttp = true;
  if (custom && !owner) binding.offlineResponse = structuredClone(plan.offlineResponse);
  if (hosted) binding.boundary = { endpoint: 'https://openrouter.ai/api/v1/chat/completions',
    settingsSha256: sha(JSON.stringify({ model: plan.model, endpoint: 'https://openrouter.ai/api/v1', provider: 'openrouter', source: 'env_local_only' })),
    pricingEvidence: structuredClone(plan.offlineHosted), textEstimatePicoUsd: 10240000, allowanceUsdCents: 500,
    inputTokenIllustration: 8192, outputTokens: 1024, hardBillingCap: false,
    billingRiskAccepted: false, liveDispatchAvailable: false, readyForExecution: false,
    retention: { remote: 'documented_provider_defaults_for_public_metadata', local: 'until_result_review' } };
  const receipt = { testPolicy: policy, bindings: { modelRehearsalPlan: plan, modelRehearsal: binding,
    sourcesAndTools: [{ path: path.relative(process.cwd(), planFile), sha256: sha(JSON.stringify(plan)) },
      ...(hosted ? [{ path: path.relative(process.cwd(), credentialFile), sha256: sha(credential) }] : []),
      ...(custom ? [{ path: path.relative(process.cwd(), responseFile), sha256: sha(response) }] : [])] },
    preparations: {}, sessions: {}, operations: {}, counters: { model: 1 }, credentialLookups: 1 };
  if (offlineHttp) {
    receipt.bindings.transport = { endpoint: 'native-owned-loopback-http/api/v1/chat/completions' };
    put('offline-http-observation.json', { version: 1, transport: 'offline_local_http', requests: 1,
      requestSha256, requestBytes: request.length, responseSha256: sha(response),
      responseBytes: Buffer.byteLength(response), status, exactRequest: true,
      fakeCredentialPresented: true, liveAuthority: false });
  }
  const requested = { evaluations: names.map(name => ({ name })) };
  for (const name of names) {
    const active = name === 'assistance-crowd';
    const preparation = { plan: { events: active ? [{ kind: 'request', body }] : [] } };
    put(`${name}/preparation/preparation.json`, preparation);
    receipt.preparations[name] = { preparationSha256: sha(JSON.stringify(preparation)), requestSha256: active ? requestSha256 : null };
    put(`${name}/service.json`, active && !hosted ? { scriptedModel: { requestModel: plan.model, responseModel: plan.observedTarget,
      provider: plan.provider, maxRequests: 1, bodySha256: requestSha256, payload: plan.input,
      response: responseValue, usage: { prompt_tokens: 7, completion_tokens: 3, total_tokens: 10 }, privacyCanaries: true } } : {});
    for (const [stage, folder] of [['preparation', 'preparation'], ['execution', 'cold']]) {
      const id = `${name}-${stage}`;
      receipt.sessions[id] = { job: name, stage };
      put(`${name}/${folder}/session.json`, { job: name, stage, model_rehearsal: structuredClone(plan) });
      put(`${name}/${folder}/inventory-final.json`, { modelRequests: active && !hosted && stage === 'execution'
        ? [{ status: 200, sha256: requestSha256, bytes: request.length, responseSha256: sha(response), responseBytes: Buffer.byteLength(response), mockCredentialPresented: true }] : [],
      wireArrivals: active && !hosted && stage === 'execution'
        ? [{ operation: 'single-model', method: 'POST', route: '/model/chat/completions', status: 200, state: 'responded' }] : [] });
    }
    put(`${name}/scoring-contract.json`, { spec: { mode: 'mock' } });
    put(`${name}/score.provisional.json`, { groundedAssistanceAccuracyCredit: 0, writableDelta: [], acceptedCertificate: false });
    if (hosted) put(`${name}/result.provisional.json`, { modelRequests: active ? 1 : 0, wireArrivals: 0 });
    const filled = active && !withheld && !rejected;
    const assisted = filled ? { model: plan.model, input_sha256: sha(canonical(plan.input)), value: responseValue.genre } : null;
    put(`${name}/cold/records.json`, [{ native: { candidate: { ...(assisted ? { assisted_genre: assisted } : {}), genre: filled ? responseValue.genre : null },
      diagnostics: active ? [{ stage: 'genre', code: rejected ? 'genre_unavailable' : withheld ? 'genre_rejected' : 'genre_assisted' }] : [] },
      serviceFailureObserved: active && rejected,
      reviewPendingAfterRestart: filled, actionableReviewAfterRestart: filled,
      persistedEvidence: { assistedGenre: assisted }, fieldObservations: { tracks: [{ genre: filled ? responseValue.genre : null }] } }]);
    if (active) files[`${name}/independent-request.json`] = request;
  }
  receipt.operations['single-model'] = { session: 'assistance-crowd-execution', category: 'model',
    dispatched: true, arrived: !hosted, completed: true,
    request_sha256: sha(JSON.stringify(['POST', '/model/chat/completions', requestSha256])),
    response: { status, bytes: Buffer.byteLength(response), sha256: sha(response),
      identityClaimsVerified: false, pricingVerified: false, usageClaimsVerified: false,
      ...(hosted ? { observation: 'offline_injected_native_response', remoteArrivalVerified: false } : {}) } };
  const http = { version: 1, transport: 'offline_local_http', requestSha256, validation: rejected ? 'rejected' : withheld ? 'withheld' : 'accepted',
    projection: rejected ? null : { model: plan.observedTarget, provider: plan.provider, genre: withheld ? '' : responseValue.genre,
      confidence: responseValue.confidence, usage: { promptTokens: usage.prompt_tokens, completionTokens: usage.completion_tokens, totalTokens: usage.total_tokens, reasoningTokens: 0 } } };
  if (offlineHttp) Object.assign(receipt.operations['single-model'].response,
    { observation: 'native_http_response', http });
  if (hosted) for (const name of names) put(`${name}/native-model-observations.json`, name === 'assistance-crowd'
    ? { 'single-model': structuredClone(receipt.operations['single-model']) } : {});
  const ownerKey = owner ? crypto.generateKeyPairSync('ed25519').publicKey.export({ type: 'spki', format: 'der' }).subarray(-32) : null;
  if (owner) {
    delete plan.offlineHosted; delete plan.offlineHttp; plan.nativePrepared = true;
    fs.writeFileSync(planFile, JSON.stringify(plan));
    receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(plan));
    delete binding.offlineFixture; delete binding.offlineHttp;
    binding.verifierSha256 = sha(ownerKey);
    files['selection.json'].modelRehearsalPlan = structuredClone(plan);
    for (const [file, value] of Object.entries(files)) if (file.endsWith('/session.json')) value.model_rehearsal = structuredClone(plan);
    Object.assign(receipt.operations['single-model'], { send_admitted: true });
    files['assistance-crowd/native-model-observations.json']['single-model'] = structuredClone(receipt.operations['single-model']);
  }
  const check = (mutate = () => {}) => {
    const current = structuredClone({ files, receipt, requested, credential, credentialMode: 0o600 }); mutate(current);
    fs.rmSync(planFile, { force: true });
    fs.writeFileSync(planFile, JSON.stringify(current.publicPlan || plan));
    if (current.planSymlink) {
      const target = path.join(root, 'plan-target.json');
      fs.renameSync(planFile, target); fs.symlinkSync(target, planFile);
    }
    if (custom) {
      fs.rmSync(responseFile, { recursive: true, force: true });
      if (current.responseKind === 'symlink') fs.symlinkSync(planFile, responseFile);
      else if (current.responseKind === 'directory') fs.mkdirSync(responseFile);
      else if (current.responseKind === 'hardlink') {
        const target = path.join(root, 'response-linked-bytes');
        fs.writeFileSync(target, response); fs.linkSync(target, responseFile);
      } else if (current.responseKind !== 'missing') fs.writeFileSync(responseFile, current.responseBytes ?? response);
    }
    if (hosted) { fs.writeFileSync(credentialFile, current.credential); fs.chmodSync(credentialFile, current.credentialMode); }
    fs.rmSync(output, { recursive: true, force: true });
    for (const [file, value] of Object.entries(current.files)) {
      const target = path.join(output, file); fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.writeFileSync(target, value instanceof Uint8Array ? value : JSON.stringify(value));
    }
    fs.writeFileSync(path.join(root, 'context.json'), JSON.stringify(current));
    return spawnSync('python3', ['-B', '-c', `import pathlib,sys,json,importlib
sys.path.insert(0,'scripts/auto-tag-eval-replay')
m=importlib.import_module('audit-supervised');r=pathlib.Path(sys.argv[1]);c=json.loads((r/'context.json').read_text())
if c.get('floatStatus'): c['receipt']['bindings']['modelRehearsalPlan']['offlineResponse']['status']=200.0
checked=m.model_rehearsal_audit(r/'output',c['receipt'],c['requested']${owner ? `,_owner_public_key=bytes.fromhex('${ownerKey.toString('hex')}')` : ''})
${owner ? `# Exercise the actual nested native checks using the independently audited raw response.
_,source=m.owner_offline_retained_auditor_source(checked,c['receipt']['bindings']['modelRehearsalPlan'])
sys.argv=['audit',str((r/'output').resolve())];ns={}
exec(source[:source.index('native = json.loads')],ns)
native=ns['adapted'];checks=native[native.index("    requests = [e['body']"):native.index("    if name.endswith('billie')")]
for expected in c['requested']['evaluations']:
    directory=r/'output'/expected['name']
    scope=dict(directory=directory,name=expected['name'],prep=m.load(directory/'preparation/preparation.json'),
        inventory=m.load(directory/'cold/inventory-final.json'),row=m.load(directory/'cold/records.json')[0],
        load=m.load,json=json,hashlib=m.hashlib)
    exec('if True:\\n'+checks,scope)
` : ''}print(json.dumps(checked))`, root], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
  };
  const resizedResponse = (c, size) => {
    c.responseBytes = response + ' '.repeat(size - Buffer.byteLength(response));
    const hash = sha(c.responseBytes);
    if (!owner) {
      c.publicPlan = c.receipt.bindings.modelRehearsalPlan;
      c.publicPlan.offlineResponse.sha256 = hash;
      c.receipt.bindings.modelRehearsal.offlineResponse.sha256 = hash;
      c.receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(c.publicPlan));
      c.files['selection.json'].modelRehearsalPlan.offlineResponse.sha256 = hash;
      for (const [file, value] of Object.entries(c.files)) if (file.endsWith('/session.json')) value.model_rehearsal.offlineResponse.sha256 = hash;
    }
    c.receipt.bindings.sourcesAndTools.at(-1).sha256 = hash;
    Object.assign(c.receipt.operations['single-model'].response, { sha256: hash, bytes: size });
    c.files['assistance-crowd/native-model-observations.json']['single-model'] = structuredClone(c.receipt.operations['single-model']);
    Object.assign(c.files['offline-http-observation.json'], { responseSha256: hash, responseBytes: size });
  };
  try {
    if (rejected) {
      const invalid = check(); assert.notEqual(invalid.status, 0, 'rejected response was credited as a completed rehearsal');
      assert.match(invalid.stderr, /rejected response cannot validate a completed rehearsal/);
      return;
    }
    const valid = check(); assert.equal(valid.status, 0, valid.stderr);
    assert.equal(JSON.parse(valid.stdout).liveAuthority, false);
    if (owner) {
      assert.equal(JSON.parse(valid.stdout).http.transport, 'offline_local_http');
      assert.equal(JSON.parse(valid.stdout).responseEvidence, custom
        ? 'public_offline_fixture_bytes_not_remote_response_evidence'
        : 'deterministic_default_fixture_reconstruction_not_retained_raw_bytes');
      assert.deepEqual(JSON.parse(valid.stdout).http.projection, http.projection);
      if (custom) {
        const boundary = check(c => resizedResponse(c, 65536));
        assert.equal(boundary.status, 0, boundary.stderr);
        const reordered = check(c => c.receipt.bindings.sourcesAndTools.reverse());
        assert.equal(reordered.status, 0, reordered.stderr);
      } else {
        // An unlisted nearby file is not a response selector, even if invalid
        // or oversized. Historical owner source paths remain accepted.
        fs.writeFileSync(responseFile, '{' + ' '.repeat(65536));
        const unlisted = check(); assert.equal(unlisted.status, 0, unlisted.stderr);
        assert.equal(JSON.parse(unlisted.stdout).responseEvidence,
          'deterministic_default_fixture_reconstruction_not_retained_raw_bytes');
        const otherSibling = check(c => {
          c.files['response-fixture.json'] = Buffer.from('{');
          c.receipt.bindings.sourcesAndTools.push({ path: path.relative(process.cwd(), path.join(output, 'response-fixture.json')), sha256: sha('{') });
        });
        assert.equal(otherSibling.status, 0, otherSibling.stderr);
        assert.equal(JSON.parse(otherSibling.stdout).responseEvidence,
          'deterministic_default_fixture_reconstruction_not_retained_raw_bytes');
      }
      const damage = [
        c => c.receipt.bindings.modelRehearsalPlan.offlineHttp = true,
        c => c.receipt.bindings.modelRehearsalPlan.offlineHosted = {},
        c => c.receipt.bindings.modelRehearsalPlan.offlineResponse = { sha256: sha(response), status },
        c => c.receipt.bindings.modelRehearsal.offlineFixture = true,
        c => c.receipt.bindings.modelRehearsal.verifierSha256 = 'a'.repeat(64),
        c => c.files['selection.json'].modelRehearsalPlan.nativePrepared = 1,
        c => c.files['disabled-enya/cold/session.json'].model_rehearsal.nativePrepared = 1,
        c => c.receipt.counters.model = true,
        c => c.receipt.credentialLookups = true,
        c => c.receipt.operations['single-model'].send_admitted = false,
        c => c.receipt.operations['single-model'].arrived = true,
        c => c.receipt.operations['single-model'].request_sha256 = requestSha256,
        c => c.receipt.operations['single-model'].response.http.transport = 'hosted_https',
        c => c.receipt.operations['single-model'].response.http.projection.genre = 'Invented',
        c => delete c.files['offline-http-observation.json'],
        c => c.files['assistance-crowd/native-model-observations.json'] = {},
        c => c.files['assistance-crowd/cold/records.json'][0].fieldObservations.tracks[0].genre = 'Invented',
        c => c.files['assistance-crowd/score.provisional.json'].groundedAssistanceAccuracyCredit = 1,
        c => c.files['assistance-crowd/score.provisional.json'].groundedAssistanceAccuracyCredit = false,
        c => c.files['assistance-crowd/score.provisional.json'].acceptedCertificate = 0,
        c => c.files['assistance-crowd/result.provisional.json'].modelRequests = true,
        c => c.files['assistance-crowd/cold/records.json'][0].serviceFailureObserved = true,
        ...(custom ? [
          ...['missing', 'symlink', 'directory', 'hardlink'].map(responseKind => c => c.responseKind = responseKind),
          c => c.responseBytes = response + ' ',
          c => resizedResponse(c, 65537),
          c => c.planSymlink = true,
          c => c.receipt.bindings.sourcesAndTools.pop(),
          c => c.receipt.bindings.sourcesAndTools.push(structuredClone(c.receipt.bindings.sourcesAndTools.at(-1))),
          c => c.receipt.bindings.sourcesAndTools.push(structuredClone(c.receipt.bindings.sourcesAndTools[0])),
          c => c.receipt.bindings.sourcesAndTools.at(-1).path = responseFile,
          c => c.receipt.bindings.sourcesAndTools.at(-1).path = './' + c.receipt.bindings.sourcesAndTools.at(-1).path,
          c => c.receipt.bindings.sourcesAndTools.at(-1).path = c.receipt.bindings.sourcesAndTools.at(-1).path.replace('/response-fixture.json', '/output/../response-fixture.json'),
          c => c.receipt.bindings.sourcesAndTools.at(-1).path = path.relative(process.cwd(), planFile),
          c => c.receipt.bindings.sourcesAndTools.at(-1).sha256 = 'f'.repeat(64),
          c => {
            c.files['response-fixture.json'] = Buffer.from(response);
            c.receipt.bindings.sourcesAndTools.at(-1).path = path.relative(process.cwd(), path.join(output, 'response-fixture.json'));
          },
          c => {
            c.files['public-native-plan.json'] = plan;
            c.files['response-fixture.json'] = Buffer.from(response);
            c.receipt.bindings.sourcesAndTools[0].path = path.relative(process.cwd(), path.join(output, 'public-native-plan.json'));
            c.receipt.bindings.sourcesAndTools.at(-1).path = path.relative(process.cwd(), path.join(output, 'response-fixture.json'));
          },
          c => c.receipt.operations['single-model'].response.status = 201,
          c => c.receipt.operations['single-model'].response.http.projection.usage.promptTokens++,
          c => c.receipt.operations['single-model'].response.remoteArrivalVerified = true,
          c => c.receipt.operations['single-model'].response.identityClaimsVerified = true,
          c => c.files['assistance-crowd/cold/records.json'][0].native.candidate.genre = 'MOCK_ACCEPTED_GENRE_CANARY',
        ] : []),
      ];
      for (const [i, mutate] of damage.entries()) assert.notEqual(check(mutate).status, 0, `accepted owner damage ${i}`);
      console.log(`${mode}: exact ${custom ? 'public response bytes' : 'default fixture'} join and ${damage.length} corruption controls; not a completion test`);
      return;
    }
    if (custom) {
      assert.equal(JSON.parse(valid.stdout).http.validation, withheld ? 'withheld' : 'accepted');
      assert.deepEqual(JSON.parse(valid.stdout).http.projection, http.projection);
      const boundary = check(c => resizedResponse(c, 65536));
      assert.equal(boundary.status, 0, boundary.stderr);
    }
    const damage = [
      c => delete c.receipt.bindings.modelRehearsalPlan,
      c => c.receipt.bindings.modelRehearsalPlan.verifier = 'injected',
      c => c.receipt.bindings.modelRehearsal.verifierSha256 = 'a'.repeat(64),
      c => c.receipt.testPolicy = 'historical',
      c => c.receipt.bindings.sourcesAndTools = [],
      c => c.receipt.bindings.sourcesAndTools[0].sha256 = 'c'.repeat(64),
      c => c.publicPlan = { ...plan, route: 'other/route' },
      c => c.files['selection.json'].model = plan.observedTarget,
      c => delete c.files['disabled-enya/cold/session.json'].model_rehearsal,
      c => {
        const prepared = c.files['assistance-crowd/preparation/preparation.json'];
        prepared.plan.events[0].body.provider.only = ['other'];
        c.receipt.preparations['assistance-crowd'].preparationSha256 = sha(JSON.stringify(prepared));
      },
      c => c.receipt.preparations['disabled-billie'].requestSha256 = requestSha256,
      ...(hosted ? [
        c => c.receipt.operations['single-model'].arrived = true,
        c => c.receipt.operations['single-model'].response.observation = 'fixture_wire',
        c => c.receipt.operations['single-model'].response.remoteArrivalVerified = true,
        c => c.receipt.bindings.modelRehearsal.offlineFixture = false,
        c => delete c.receipt.bindings.modelRehearsal.offlineFixture,
        c => {
          delete c.receipt.bindings.modelRehearsal.offlineFixture;
          c.receipt.bindings.offlineFixture = true;
        },
        c => c.receipt.bindings.sourcesAndTools.pop(),
        c => c.receipt.bindings.sourcesAndTools.push(c.receipt.bindings.sourcesAndTools[1]),
        c => c.credential += '# drift\n',
        c => c.credentialMode = 0o644,
        c => c.receipt.bindings.modelRehearsal.boundary.pricingEvidence.promptPicoUsd++,
        c => c.receipt.bindings.modelRehearsal.boundary.textEstimatePicoUsd++,
        c => c.receipt.bindings.modelRehearsal.boundary.liveDispatchAvailable = true,
        c => c.receipt.bindings.modelRehearsal.policy = 'offline-model-rehearsal-v1;separate-verifier;fake-credential;loopback-only;no-live;no-retry;no-repair',
        c => c.receipt.bindings.modelRehearsal.verifierSha256 = sha(Buffer.from('197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61', 'hex')),
        c => c.receipt.bindings.modelRehearsal.verifierSha256 = sha(Buffer.from('d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a', 'hex')),
        c => c.files['assistance-crowd/native-model-observations.json'] = {},
        c => c.files['disabled-enya/native-model-observations.json'] = c.files['assistance-crowd/native-model-observations.json'],
        c => c.files['assistance-crowd/native-model-observations.json']['single-model'].completed = false,
        c => c.files['assistance-crowd/native-model-observations.json']['single-model'].completed = 1,
        c => c.files['assistance-crowd/service.json'].scriptedModel = {},
        c => c.files['assistance-crowd/cold/inventory-final.json'].wireArrivals.push({ operation: 'single-model', method: 'POST' }),
        c => c.files['assistance-crowd/result.provisional.json'].modelRequests = 0,
        c => c.files['assistance-crowd/result.provisional.json'].wireArrivals = 1,
        c => c.receipt.bindings.sourcesAndTools[1].sha256 = 'f'.repeat(64),
        ...[null, {}, { ...plan.offlineHosted, extra: 1 }, { ...plan.offlineHosted, promptPicoUsd: true },
          { ...plan.offlineHosted, promptPicoUsd: -1 }, { ...plan.offlineHosted, completionPicoUsd: 0.5 },
          { ...plan.offlineHosted, promptPicoUsd: 2 ** 64 }, { ...plan.offlineHosted, completionPicoUsd: 5e12 },
          { ...plan.offlineHosted, catalogSha256: 'not-a-digest' }].map(pricing => c => {
          c.publicPlan = c.receipt.bindings.modelRehearsalPlan;
          c.publicPlan.offlineHosted = pricing;
          c.receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(c.publicPlan));
          c.receipt.bindings.modelRehearsal.boundary.pricingEvidence = pricing;
          if (pricing) c.receipt.bindings.modelRehearsal.boundary.textEstimatePicoUsd = pricing.promptPicoUsd * 8192 + pricing.completionPicoUsd * 1024;
        }),
      ] : [
        c => c.files['assistance-crowd/service.json'].scriptedModel.responseModel = plan.model,
        c => c.files['assistance-crowd/cold/inventory-final.json'].modelRequests[0].responseSha256 = 'b'.repeat(64),
      ]),
      c => c.receipt.operations['single-model'].response.identityClaimsVerified = true,
      ...(!offlineHttp ? [c => Object.assign(c.receipt.operations['single-model'].response,
        { observation: 'native_http_response', remoteArrivalVerified: false, http: {
          version: 1, transport: 'offline_local_http', requestSha256, validation: 'accepted',
          projection: { model: plan.observedTarget, provider: plan.provider, genre: responseValue.genre,
            confidence: 0.8, usage: { promptTokens: 7, completionTokens: 3, totalTokens: 10, reasoningTokens: 0 } } } })] : []),
      ...(offlineHttp ? [
        c => delete c.receipt.bindings.modelRehearsal.offlineHttp,
        c => delete c.receipt.bindings.modelRehearsalPlan.offlineHttp,
        c => { delete c.receipt.bindings.modelRehearsalPlan.offlineHttp; delete c.receipt.bindings.modelRehearsal.offlineHttp; },
        c => delete c.receipt.bindings.modelRehearsalPlan.offlineHosted,
        c => delete c.files['assistance-crowd/cold/session.json'].model_rehearsal.offlineHttp,
        c => c.receipt.bindings.transport.endpoint = 'http://127.0.0.1:1234/api/v1/chat/completions',
        c => c.receipt.operations['single-model'].response.observation = 'offline_injected_native_response',
        c => delete c.receipt.operations['single-model'].response.http,
        c => c.receipt.operations['single-model'].response.sha256 = 'e'.repeat(64),
        c => c.receipt.operations['single-model'].response.bytes++,
        c => c.receipt.operations['single-model'].response.http.transport = 'hosted_https',
        c => c.receipt.operations['single-model'].response.http.projection.genre = 'Invented',
        c => c.receipt.operations['single-model'].response.http.projection.usage.promptTokens++,
        c => c.receipt.operations['single-model'].response.http.requestSha256 = 'd'.repeat(64),
        c => c.receipt.operations['single-model'].response.http.validation = withheld ? 'accepted' : 'withheld',
        c => delete c.files['offline-http-observation.json'],
        c => {
          c.publicPlan = { ...plan, offlineHttp: 1 };
          c.receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(c.publicPlan));
        },
        c => c.files['selection.json'].modelRehearsalPlan.offlineHttp = 1,
        ...names.flatMap(name => ['preparation', 'cold'].map(folder => c => c.files[`${name}/${folder}/session.json`].model_rehearsal.offlineHttp = 1)),
        c => {
          c.publicPlan = c.receipt.bindings.modelRehearsalPlan;
          delete c.publicPlan.offlineHttp; delete c.receipt.bindings.modelRehearsal.offlineHttp;
          delete c.files['selection.json'].modelRehearsalPlan.offlineHttp;
          for (const [file, value] of Object.entries(c.files)) if (file.endsWith('/session.json')) delete value.model_rehearsal.offlineHttp;
          c.receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(c.publicPlan));
        },
        ...[false, 1, 'true', null].flatMap(value => [
          c => c.receipt.bindings.modelRehearsal.offlineHttp = value,
          c => {
            c.publicPlan = c.receipt.bindings.modelRehearsalPlan;
            c.publicPlan.offlineHttp = value;
            c.receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(c.publicPlan));
          },
        ]),
        ...Object.entries({ version: true, transport: 'hosted_https', requests: 2,
          requestSha256: 'a'.repeat(64), requestBytes: request.length + 1,
          responseSha256: 'b'.repeat(64), responseBytes: Buffer.byteLength(response) + 1,
          status: 201, exactRequest: false, fakeCredentialPresented: false, liveAuthority: true,
          authorization: 'MOCK_ONLY_SECRET_CANARY' }).map(([key, value]) => c => c.files['offline-http-observation.json'][key] = value),
        ...['requests', 'requestBytes', 'responseBytes', 'status'].flatMap(key => [
          c => c.files['offline-http-observation.json'][key] = true,
          c => delete c.files['offline-http-observation.json'][key],
        ]),
      ] : [
        c => c.receipt.bindings.modelRehearsal.offlineHttp = true,
        c => c.receipt.bindings.modelRehearsalPlan.offlineHttp = true,
      ]),
      c => c.receipt.operations.extra = structuredClone(c.receipt.operations['single-model']),
      c => c.files['disabled-billie/cold/inventory-final.json'].modelRequests.push({}),
      ...(owner ? [
        c => c.files['assistance-crowd/cold/records.json'][0].reviewPendingAfterRestart = withheld,
        c => c.files['assistance-crowd/cold/records.json'][0].actionableReviewAfterRestart = withheld,
        c => {
          const row = c.files['assistance-crowd/cold/records.json'][0];
          row.reviewPendingAfterRestart = row.actionableReviewAfterRestart = withheld;
        },
      ] : []),
      ...(!withheld ? [c => c.files['assistance-crowd/cold/records.json'][0].native.candidate.assisted_genre.model = plan.observedTarget] : [
        c => c.files['assistance-crowd/cold/records.json'][0].native.candidate.assisted_genre = { value: 'Invented' },
        c => c.files['assistance-crowd/cold/records.json'][0].native.candidate.genre = 'Invented',
      ]),
      c => c.files['assistance-crowd/cold/records.json'][0].fieldObservations.tracks[0].genre = 'wrong',
      c => c.files['assistance-crowd/scoring-contract.json'].spec.mode = 'live',
      c => c.files['assistance-crowd/score.provisional.json'].groundedAssistanceAccuracyCredit = 1,
      c => c.files['assistance-crowd/score.provisional.json'].acceptedCertificate = true,
      c => c.files['assistance-crowd/score.provisional.json'].writableDelta = [{}],
      ...(custom ? [
        ...['missing', 'symlink', 'directory'].map(responseKind => c => c.responseKind = responseKind),
        c => c.responseBytes = response + ' ',
        c => resizedResponse(c, 65537),
        c => c.planSymlink = true,
        c => c.floatStatus = true,
        c => {
          c.files['public-plan.json'] = plan;
          c.receipt.bindings.sourcesAndTools[0].path = path.relative(process.cwd(), path.join(output, 'public-plan.json'));
        },
        c => c.receipt.bindings.sourcesAndTools.pop(),
        c => c.receipt.bindings.sourcesAndTools.push(structuredClone(c.receipt.bindings.sourcesAndTools.at(-1))),
        c => c.receipt.bindings.sourcesAndTools.push(structuredClone(c.receipt.bindings.sourcesAndTools[0])),
        c => c.receipt.bindings.sourcesAndTools.at(-1).path = responseFile,
        c => c.receipt.bindings.sourcesAndTools.at(-1).path = './' + c.receipt.bindings.sourcesAndTools.at(-1).path,
        c => c.receipt.bindings.sourcesAndTools.at(-1).path = path.relative(process.cwd(), planFile),
        c => c.receipt.bindings.sourcesAndTools.at(-1).sha256 = 'f'.repeat(64),
        c => delete c.receipt.bindings.modelRehearsal.offlineResponse,
        c => c.receipt.bindings.modelRehearsal.offlineResponse.status = 201,
        c => c.files['selection.json'].modelRehearsalPlan.offlineResponse.status = true,
        c => c.files['disabled-enya/cold/session.json'].model_rehearsal.offlineResponse.status = true,
        c => c.receipt.counters.model = true,
        c => c.receipt.credentialLookups = true,
        c => c.files['assistance-crowd/result.provisional.json'].modelRequests = true,
        c => c.files['assistance-crowd/result.provisional.json'].wireArrivals = false,
        c => c.files['assistance-crowd/score.provisional.json'].groundedAssistanceAccuracyCredit = false,
        c => c.files['assistance-crowd/score.provisional.json'].acceptedCertificate = 0,
        c => c.receipt.operations['single-model'].completed = false,
        c => c.files['assistance-crowd/cold/records.json'][0].native.diagnostics[0].code = 'genre_unavailable',
        ...[null, {}, { sha256: sha(response).toUpperCase(), status }, { sha256: sha(response), status: true },
          { sha256: sha(response), status: 99 }, { sha256: sha(response), status: 600 },
          { sha256: sha(response), status: '200' }, { sha256: sha(response), status, path: 'arbitrary' }]
          .map(value => c => {
            c.publicPlan = c.receipt.bindings.modelRehearsalPlan;
            c.publicPlan.offlineResponse = value;
            c.receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(c.publicPlan));
          }),
      ] : []),
    ];
    for (const [i, mutate] of damage.entries()) assert.notEqual(check(mutate).status, 0, `accepted semantic damage ${i}`);
    console.log(`${mode}: one accepted join and ${damage.length} rejected semantic corruptions`);
    const historical = check(c => {
      c.receipt.bindings = {}; delete c.receipt.testPolicy;
      c.files['selection.json'] = { model: 'scripted/genre-v1' };
      for (const [file, value] of Object.entries(c.files)) if (file.endsWith('/session.json')) delete value.model_rehearsal;
    });
    assert.equal(historical.status, 0, historical.stderr);
    assert.equal(JSON.parse(historical.stdout), null);
  } finally { fs.rmSync(root, { recursive: true }); }
});

test('non-fixed retained audit counts adaptations and derives acceptance totals without canary credit', () => {
  const { spawnSync } = require('node:child_process');
  const result = spawnSync('python3', ['-B', '-c', `
import pathlib,sys,json,importlib,hashlib
sys.path.insert(0,'scripts/auto-tag-eval-replay')
m=importlib.import_module('audit-supervised')
for args in [(False,False,True),(True,False,True)]:
    try: m.retained_auditor_source(*args)
    except AssertionError: pass
    else: raise AssertionError('accepted non-HTTP response adaptation')
original,source=m.retained_auditor_source(True,True,True)
compile(source,'response-durable','exec')
ns={};sys.argv=['audit',str(pathlib.Path('.amp/in/artifacts').resolve())]
exec(source[:source.index('native = json.loads')],ns)
native=ns['adapted'];compile(native,'response-native','exec')
# The sealed sources contain no response canary or usage constant. Their
# assumptions are omitted provenance and zero accepted user readbacks.
assert source.count('assert source.count(')==9
assert "row['native']['candidate'].get('assisted_genre')" in native
scope=dict(row={'persistedEvidence':{'assistedGenre':None},'native':{'candidate':{}}})
line=next(l.strip() for l in native.splitlines() if "assert row['persistedEvidence']['assistedGenre']" in l)
exec(line,scope)
score={'userAcceptance':{'agreeingFieldSlots':2},'fields':[{'userAcceptanceAgreement':True},{'userAcceptanceAgreement':True}]}
line=next(l.strip() for l in native.splitlines() if "assert score['userAcceptance']['agreeingFieldSlots']" in l)
exec(line,dict(score=score))
score['userAcceptance']['agreeingFieldSlots']=3
try: exec(line,dict(score=score))
except AssertionError: pass
else: raise AssertionError('forged acceptance total admitted')
assert "userAcceptanceAgreeing=sum(" in native
assert "score = scorer.project_native(root, contract, records)" in native
assert "score == load(directory / 'score.provisional.json')" in native
print(json.dumps(dict(responseAdaptations=3,durableSha256=hashlib.sha256(source.encode()).hexdigest(),nativeSha256=hashlib.sha256(native.encode()).hexdigest())))
`], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  console.log(result.stdout.trim());
});

for (const transport of ['hosted', 'HTTP', 'owner offline', 'owner withheld']) test(`${transport} retained auditor adaptations reconcile 497 settled operations with only 496 fixture arrivals`, () => {
  const offlineHttp = transport !== 'hosted', owner = transport.startsWith('owner ');
  const { spawnSync } = require('node:child_process');
  const result = spawnSync('python3', ['-B', '-c', `
import pathlib,sys,json,importlib,tempfile,hashlib,copy
sys.path.insert(0,'scripts/auto-tag-eval-replay')
m=importlib.import_module('audit-supervised')
offline_http=${offlineHttp ? 'True' : 'False'}
assert hashlib.sha256(m.retained_auditor_source()[1].encode()).hexdigest()=='540699a2153af67f86f845eebecaed879164908fdc35644284be6043aee3a0f1'
assert hashlib.sha256(m.retained_auditor_source(True)[1].encode()).hexdigest()=='319be49ed4c0156723624b1934aac9256b4185f3c0af820ba6e875cb1c31f67a'
assert hashlib.sha256(m.retained_auditor_source(True,True)[1].encode()).hexdigest()=='f719b608d79b08c76c42006dee7f9dc0caec4c4716fda41372416ab9595eb680'
plan=dict(model='~deepseek/alias',input=dict(artist='public artist'))
genre=${transport === 'owner withheld' ? "''" : "'Jazz'"}
rehearsal=dict(model=plan['model'],http=dict(validation='withheld' if not genre else 'accepted',projection=dict(genre=genre)))
original,source=m.owner_offline_retained_auditor_source(rehearsal,plan) if ${owner ? 'True' : 'False'} else (m.retained_auditor_source(True,offline_http=True) if offline_http else m.retained_auditor_source(True))
compile(source,'adapted-durable','exec')
# Obtain the exact nested native adaptation without executing either auditor.
ns={}
sys.argv=['audit',str(pathlib.Path('.amp/in/artifacts').resolve())]
exec(source[:source.index('native = json.loads')],ns)
native=ns['adapted'];compile(native,'adapted-native','exec')
wire_checks=source[source.index('all_wires = []'):source.index('print(json.dumps')]
request_checks=native[native.index("    requests = [e['body']"):native.index("    if name.endswith('billie')")]
summary=native[native.index('    audit.append(dict'):native.index('for rows in pairs.values():')]
digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
load=lambda p:json.loads(p.read_text())
with tempfile.TemporaryDirectory(dir='.amp/in/artifacts',prefix='hosted-wire-audit-') as temp:
    run=pathlib.Path(temp).resolve()
    put=lambda p,v:(p.parent.mkdir(parents=True,exist_ok=True),p.write_text(json.dumps(v)))
    names=['disabled-billie','disabled-enya','disabled-crowd','assistance-billie','assistance-enya','assistance-crowd']
    requested={'evaluations':[{'name':n} for n in names]}
    receipt={'sessions':{},'operations':{},'preparations':{},'counters':dict(provider=472,inventory=24,model=1,reserved=497,arrived=496,completed=497)}
    request_bytes=json.dumps({'model':'alias'},sort_keys=True,separators=(',',':')).encode()
    request_sha=hashlib.sha256(request_bytes).hexdigest()
    model={'category':'model','session':'assistance-crowd-execution','dispatched':True,'arrived':False,'completed':True,
        'request_sha256':hashlib.sha256(json.dumps(['POST','/model/chat/completions',request_sha],separators=(',',':')).encode()).hexdigest(),
        'response':{'observation':'native_http_response' if offline_http else 'offline_injected_native_response','remoteArrivalVerified':False}}
    if ${owner ? 'True' : 'False'}: model.update(send_admitted=True)
    receipt['operations']['model']=model
    for index,name in enumerate(names):
        directory=run/name
        prep={'plan':{'events':[{'kind':'request','body':{'model':'alias'}}] if name=='assistance-crowd' else []}}
        put(directory/'preparation/preparation.json',prep)
        receipt['preparations'][name]={'preparationSha256':digest(directory/'preparation/preparation.json')}
        (directory/'independent-request.json').write_bytes(request_bytes)
        put(directory/'native-model-observations.json',{'model':model} if name=='assistance-crowd' else {})
        for stage,folder in [('preparation','preparation'),('execution','cold')]:
            sid=name+'-'+stage
            put(directory/folder/'cold.jsonl',{})
            receipt['sessions'][sid]=dict(job=name,stage=stage,attached=True,sealed=True,finished=True,closed=True,
                native_live=False,fixture_live=False,observation_sha256=digest(directory/folder/'cold.jsonl'))
            count=40 if index<2 else 39
            wires=[]
            for i in range(count+2):
                oid=sid+'-'+str(i);category='provider' if i<count else 'inventory'
                receipt['operations'][oid]=dict(session=sid,category=category,dispatched=True,arrived=True,completed=True)
                wires.append(dict(operation=oid,method='GET',route='/provider' if i<count else '/__fixtures/compact'))
            inv=dict(compact=True,records=[{}]*1533,misses=[],requests=[{}]*count,modelRequests=[],wireArrivals=wires)
            put(directory/folder/'inventory-final.json',inv);put(directory/(folder+'-fixture-ingress.json'),inv)
    put(run/'wire-reconciliation.json',dict(counts=receipt['counters'],nativeModelObservations=1,actualWireArrivals=496,uniqueOperations=497))
    def check():
        scope=dict(run=run,receipt=receipt,requested=requested,load=load,digest=digest,json=json)
        exec(wire_checks,scope)
        assert scope['model']==1 and len(scope['all_wires'])==496
        audit=[]
        for name in names:
            directory=run/name;prep=load(directory/'preparation/preparation.json');inventory=load(directory/'cold/inventory-final.json')
            assisted=dict(model=plan['model'],input_sha256=hashlib.sha256(json.dumps(plan['input'],ensure_ascii=False,sort_keys=True,separators=(',',':')).encode()).hexdigest(),value=genre) if genre and ${owner ? 'True' : 'False'} else None
            review=bool(genre) if ${owner ? 'True' : 'False'} else True
            row=dict(reviewPendingAfterRestart=review,actionableReviewAfterRestart=review,persistedEvidence={'assistedGenre':assisted},native={'candidate':{'assisted_genre':assisted} if assisted else {}},outcome='test',pairedMock={'matchedEvents':[]},elapsedMs=0)
            if not ${owner ? 'True' : 'False'}: row['native']['candidate']['assisted_genre']=None
            prep['plan']['variant']='test'
            context=dict(directory=directory,name=name,prep=prep,inventory=inventory,row=row,load=load,json=json,hashlib=hashlib,audit=audit,inv_file=directory/'cold/inventory-final.json')
            exec('if True:\\n'+request_checks,context);exec('if True:\\n'+summary,context)
        assert sum(a['modelRequests'] for a in audit)==1
        assert sum(a['wireArrivals'] for a in audit)==248
    check()
    def rejected(change,restore):
        change()
        try: check()
        except (AssertionError,KeyError): pass
        else: raise AssertionError('accepted corrupted hosted wire evidence')
        finally: restore()
    rejected(lambda:model.update(arrived=True),lambda:model.update(arrived=False))
    if ${owner ? 'True' : 'False'}:
        rejected(lambda:model.update(send_admitted=False),lambda:model.update(send_admitted=True))
        reserved_sha=model['request_sha256']
        rejected(lambda:model.update(request_sha256=request_sha),lambda:model.update(request_sha256=reserved_sha))
    rejected(lambda:receipt['counters'].update(arrived=497),lambda:receipt['counters'].update(arrived=496))
    rejected(lambda:receipt['operations'].update(extra=copy.deepcopy(model)),lambda:receipt['operations'].pop('extra'))
    observation=model['response']['observation']
    rejected(lambda:model['response'].update(observation='offline_injected_native_response' if offline_http else 'native_http_response'),lambda:model['response'].update(observation=observation))
    projection=run/'assistance-crowd/native-model-observations.json'
    rejected(lambda:put(projection,{}),lambda:put(projection,{'model':model}))
    wirefile=run/'wire-reconciliation.json';wire=load(wirefile)
    rejected(lambda:put(wirefile,dict(wire,actualWireArrivals=497)),lambda:put(wirefile,wire))
    invfile=run/'assistance-crowd/cold/inventory-final.json';snapshot=run/'assistance-crowd/cold-fixture-ingress.json';inv=load(invfile)
    invented=copy.deepcopy(inv);invented['modelRequests']=[{}];invented['wireArrivals'].append(dict(operation='model',method='POST',route='/model/chat/completions'))
    rejected(lambda:(put(invfile,invented),put(snapshot,invented)),lambda:(put(invfile,inv),put(snapshot,inv)))
print('${transport} counted native/durable adaptations and ${owner ? 9 : 7} corruption controls passed')
`], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /corruption controls passed/);
  console.log(result.stdout.trim());
});

test('completed owner retained native checks derive strict review and provenance from the audited response', () => {
  const { spawnSync } = require('node:child_process');
  const result = spawnSync('python3', ['-B', '-c', `
import pathlib,sys,json,importlib,hashlib,copy
sys.path.insert(0,'scripts/auto-tag-eval-replay')
m=importlib.import_module('audit-supervised')
plan=dict(model='~deepseek/alias',input=dict(artist='public artist',album='public album'))
canonical=lambda v:json.dumps(v,ensure_ascii=False,sort_keys=True,separators=(',',':')).encode()
sha=lambda b:hashlib.sha256(b).hexdigest()
body=dict(model=plan['model'])
class Directory:
    def __truediv__(self, name): return self
    def read_bytes(self): return canonical(body)
directory=Directory()
operation=dict(request_sha256=sha(json.dumps(['POST','/model/chat/completions',sha(canonical(body))],separators=(',',':')).encode()))
accepted=0;rejected=0
for genre,confidence in [('',0.59),('',0.8),('Jazz',0.8),('Soul',0.91)]:
    rehearsal=dict(model=plan['model'],http=dict(validation='accepted' if genre else 'withheld',projection=dict(genre=genre,confidence=confidence)))
    _,source=m.owner_offline_retained_auditor_source(rehearsal,plan)
    ns={};sys.argv=['audit',str(pathlib.Path('.amp/in/artifacts').resolve())]
    exec(source[:source.index('native = json.loads')],ns)
    native=ns['adapted']
    checks=native[native.index("    requests = [e['body']"):native.index("    if name.endswith('billie')")]
    assisted=dict(model=plan['model'],input_sha256=sha(canonical(plan['input'])),value=genre) if genre else None
    row=dict(reviewPendingAfterRestart=bool(genre),actionableReviewAfterRestart=bool(genre),
        persistedEvidence=dict(assistedGenre=assisted),native=dict(candidate=dict(assisted_genre=assisted) if genre else {}))
    def check(value):
        scope=dict(directory=directory,name='assistance-crowd',prep=dict(plan=dict(events=[dict(kind='request',body=body)])),
            inventory=dict(modelRequests=[]),row=value,load=lambda p:{'one':operation},json=json,hashlib=hashlib)
        exec('if True:\\n'+checks,scope)
    check(row);accepted+=1
    if not genre:
        explicit_null=copy.deepcopy(row);explicit_null['native']['candidate']['assisted_genre']=None
        check(explicit_null);accepted+=1
    def deny(value):
        global rejected
        try: check(value)
        except (AssertionError,KeyError): rejected+=1
        else: raise AssertionError('accepted inconsistent completed owner review/provenance')
    for pending,actionable in [(False,False),(False,True),(True,False),(True,True),(0,0),(1,1),(None,None)]:
        if type(pending) is bool and pending is actionable is bool(genre): continue
        deny(dict(row,reviewPendingAfterRestart=pending,actionableReviewAfterRestart=actionable))
    for key in ('reviewPendingAfterRestart','actionableReviewAfterRestart'):
        changed=copy.deepcopy(row);del changed[key];deny(changed)
    for provenance in (None,dict(model=plan['model'],input_sha256='0'*64,value='Invented'),
        dict(model='observed target',input_sha256=sha(canonical(plan['input'])),value=genre),
        dict(model=plan['model'],input_sha256=sha(canonical(plan['input'])),value=genre,extra=True)):
        if provenance==assisted: continue
        changed=copy.deepcopy(row)
        changed['persistedEvidence']['assistedGenre']=provenance
        changed['native']['candidate']['assisted_genre']=provenance
        deny(changed)
    changed=copy.deepcopy(row);del changed['persistedEvidence']['assistedGenre'];deny(changed)
    if genre:
        changed=copy.deepcopy(row);del changed['native']['candidate']['assisted_genre'];deny(changed)
try: m.owner_offline_retained_auditor_source(dict(model=plan['model'],http=dict(validation='rejected',projection=None)),plan)
except AssertionError: rejected+=1
else: raise AssertionError('rejected response admitted to completed owner adaptations')
print(f'completed owner response-derived checks: {accepted} valid variants and {rejected} corruption controls passed')
`], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /corruption controls passed/);
  console.log(result.stdout.trim());
});

test('recovery records observed receiver success, never staged files or eventual supervisor exit', () => {
  const { validateOutcome } = require('./run-supervised.cjs');
  const config = { generation: 'fresh', receiverPid: 123, receiverGroup: 321 };
  const status = { pid: 123, status: 0, signal: null };
  const value = { state: 'complete', authority: 'native_witness_and_timely_supervisor_exit',
    generation: 'fresh', receiverPid: 123, receiverGroup: 321, supervisorGroup: 987,
    executionGroup: 789, supervisorExitObservedAt: 190, acceptedAt: 191, deadline: 200,
    requestedFieldSlots: 216, liveAuthority: false, controlOnly: false,
    evidenceSha256: 'a'.repeat(64), resultSha256: 'b'.repeat(64) };
  assert.doesNotThrow(() => validateOutcome(status, config, value));
  for (const mutate of [r => r.state = 'provisional_complete', r => r.generation = 'old',
    r => r.acceptedAt = 200, r => r.supervisorExitObservedAt = 201,
    r => r.supervisorExitObservedAt = null, r => r.executionGroup = 321,
    r => r.resultSha256 = null, r => r.controlOnly = true, r => delete r.controlOnly]) {
    const changed = structuredClone(value); mutate(changed);
    assert.throws(() => validateOutcome(status, config, changed));
  }
  for (const change of [{ status: 78 }, { signal: 'SIGKILL' }, { pid: 124 }, { error: Error('unknown') }]) {
    assert.throws(() => validateOutcome({ ...status, ...change }, config, value));
  }
});

test('launcher returns incomplete without waiting for inherited descendant pipes', async () => {
  const fs = require('node:fs'), path = require('node:path');
  const { spawn } = require('node:child_process');
  const { once } = require('node:events');
  const root = fs.mkdtempSync(path.resolve('.amp/in/launcher-pipe-'));
  const executable = path.join(root, 'fixture.cjs');
  // Finite disposable descendant: no cleanup signal or stored PID is needed.
  const exiting = path.join(root, 'exiting.json');
  fs.writeFileSync(executable, `#!${process.execPath}\nrequire('node:child_process').spawn(process.execPath,
    ['-e', 'setTimeout(() => {}, 2500)'], {stdio: 'inherit'});
    require('node:fs').writeFileSync(${JSON.stringify(exiting)}, String(Date.now())); process.exit(78);\n`, { mode: 0o700 });
  const child = spawn(process.execPath, [require.resolve('./run-supervised.cjs'), path.join(root, 'output'), executable],
    { env: { PATH: process.env.PATH }, stdio: 'ignore' });
  const [status] = await once(child, 'exit');
  const elapsed = Date.now() - Number(fs.readFileSync(exiting, 'utf8'));
  try {
    assert.equal(status, 1);
    assert.ok(elapsed < 1500, `descendant pipe delayed incomplete return: ${elapsed}ms`);
  } finally {
    fs.rmSync(root, { recursive: true });
  }
});

test('failed log storage stops a silent driver without waiting for another chunk', async () => {
  const fs = require('node:fs'), path = require('node:path');
  const { spawn } = require('node:child_process');
  const { once } = require('node:events');
  const root = fs.mkdtempSync(path.resolve('.amp/in/log-failure-'));
  const executable = path.join(root, 'failed-sink.cjs');
  const entered = path.join(root, 'failure-entered');
  fs.writeFileSync(executable, `#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(entered)}, 'entered'); process.exit(78);\n`, { mode: 0o700 });
  const child = spawn(process.execPath, ['-e', `
    const fs = require('node:fs');
    // Test the response to an actual failure, not helper startup latency.
    const watch = fs.watch(${JSON.stringify(root)}, () => {
      if (fs.existsSync(${JSON.stringify(entered)})) {
        watch.close();
        setTimeout(() => process.exit(0), 1500);
      }
    });
    require('./scripts/auto-tag-eval-replay/bounded-storage.cjs').logSink(process.env.SOUNDROBE_STORAGE_TARGET);
  `], { env: { SOUNDROBE_STORAGE_REQUIRED: '1', SOUNDROBE_NATIVE_OUTPUT: root,
    SOUNDROBE_NATIVE_GENERATION: 'test-generation', SOUNDROBE_STORAGE_EXECUTABLE: executable,
    SOUNDROBE_STORAGE_TARGET: path.join(root, 'native.log') }, stdio: 'ignore' });
  try { assert.equal((await once(child, 'exit'))[0], 78); }
  finally { fs.rmSync(root, { recursive: true }); }
});

test('incomplete audit retains all slots when storage recording is unavailable', () => {
  const fs = require('node:fs'), path = require('node:path');
  const { spawnSync } = require('node:child_process');
  const root = fs.mkdtempSync(path.resolve('.amp/in/storage-missing-'));
  try {
    fs.copyFileSync('test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json', path.join(root, 'requested-membership.json'));
    fs.writeFileSync(path.join(root, 'native-supervision.json'), '{}');
    const project = () => {
      const result = spawnSync('python3', ['-B', '-c',
        `import pathlib,sys,json;sys.path.insert(0,'scripts/auto-tag-eval-replay');from importlib import import_module;print(json.dumps(import_module('audit-supervised').incomplete_audit(pathlib.Path(sys.argv[1]))))`,
        root], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
      assert.equal(result.status, 0, result.stderr);
      const audit = JSON.parse(result.stdout);
      assert.equal(audit.slots.length, 216);
      assert.equal(new Set(audit.slots.map(s => s.id)).size, 216);
      assert.equal(audit.unavailableFieldSlots, 216);
      assert.equal(audit.validatedFieldSlots, 0);
      assert.equal(audit.storage.state, 'unknown');
      assert.ok(audit.slots.every(s => s.effects === 'unknown' && s.observation === 'unavailable'));
      return audit;
    };
    assert.equal(project().membershipEvidence, 'matched');
    fs.unlinkSync(path.join(root, 'native-supervision.json'));
    fs.unlinkSync(path.join(root, 'requested-membership.json'));
    assert.equal(project().membershipEvidence, 'unavailable');
    fs.writeFileSync(path.join(root, 'native-supervision.json'), '{');
    fs.writeFileSync(path.join(root, 'requested-membership.json'), '{}');
    fs.writeFileSync(path.join(root, '.storage-ledger'), 'torn');
    assert.equal(project().membershipEvidence, 'mismatched');
  } finally { fs.rmSync(root, { recursive: true }); }
});

test('read-only recovery reconstructs source bindings and rejects missing or corrupt split state', () => {
  const fs = require('node:fs'), path = require('node:path');
  const { execFileSync } = require('node:child_process');
  const { recovered } = require('./check-mock-authority.cjs');
  const root = fs.mkdtempSync(path.resolve('.amp/in/binding-recovery-'));
  const cases = ['split', 'legacy', 'missing', 'malformed', 'malformed_hashed', 'changed', 'reference', 'inline', 'table', 'uncommitted'];
  try {
    execFileSync('python3', ['-B', '-c', `import sqlite3,json,hashlib,pathlib,sys
root=pathlib.Path(sys.argv[1])
for case in json.loads(sys.argv[2]):
    directory=root/case/'authority'
    directory.mkdir(parents=True)
    c=sqlite3.connect(directory/'registry.sqlite')
    c.executescript('CREATE TABLE authority (id INTEGER PRIMARY KEY,state TEXT); CREATE TABLE authority_bindings (id INTEGER PRIMARY KEY,value TEXT);')
    binding='{"executable":"independent-source-binding","supervision":{"generation":"fresh"}}'
    state=dict(bindings=None,bindings_sha256=hashlib.sha256(binding.encode()).hexdigest(),operations={},state='preparing')
    if case=='legacy': state['bindings']=json.loads(binding); del state['bindings_sha256']
    if case=='reference': del state['bindings_sha256']
    if case=='inline': state['bindings']={}
    if case=='malformed_hashed': state['bindings_sha256']=hashlib.sha256(b'{').hexdigest()
    if case!='uncommitted': c.execute('INSERT INTO authority VALUES (1,?)',(json.dumps(state),))
    if case not in ('missing','uncommitted'): c.execute('INSERT INTO authority_bindings VALUES (1,?)',('{' if case in ('malformed','malformed_hashed') else '{}' if case=='changed' else binding,))
    if case in ('table','legacy'): c.execute('DROP TABLE authority_bindings')
    c.commit()
    c.close()`, root, JSON.stringify(cases)], { env: { PATH: process.env.PATH } });
    for (const name of cases) {
      const directory = path.join(root, name), registry = path.join(directory, 'authority/registry.sqlite');
      const before = fs.readFileSync(registry);
      if (name === 'split' || name === 'legacy') {
        assert.deepEqual(recovered(directory).bindings,
          { executable: 'independent-source-binding', supervision: { generation: 'fresh' } });
      } else assert.throws(() => recovered(directory), `accepted ${name} source bindings`);
      assert.deepEqual(fs.readFileSync(registry), before, 'read-only export modified evidence');
    }
  } finally { fs.rmSync(root, { recursive: true }); }
});

test('storage audit validates issuance history, terminal reserve and both original caps', () => {
  const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
  const { spawnSync } = require('node:child_process');
  const root = fs.mkdtempSync(path.resolve('.amp/in/storage-audit-'));
  const generation = '12345678-1234-1234-1234-123456789012';
  const record = (sequence, growth, writes, terminal, ticketGrowth, ticketWrites) => {
    const bytes = Buffer.alloc(256);
    Buffer.from(generation.replaceAll('-', ''), 'hex').copy(bytes, 1);
    crypto.createHash('sha256').update(root).digest().copy(bytes, 17);
    for (const [offset, value] of [[49,512*1048576], [57,1024*1048576], [65,growth],
      [73,writes], [81,sequence], [91,ticketGrowth], [99,ticketWrites]]) bytes.writeBigUInt64LE(BigInt(value), offset);
    bytes[89] = Number(terminal); bytes.writeUInt32LE(123,107); bytes[111] = 0;
    bytes.write('mock-storage-v1\0',112);
    crypto.createHash('sha256').update(bytes.subarray(1,224)).digest().copy(bytes,224);
    return bytes;
  };
  const check = (allowStopped = false) => spawnSync('python3', ['-B','-c',
    `import pathlib,sys;sys.path.insert(0,'scripts/auto-tag-eval-replay');from importlib import import_module;print(import_module('audit-supervised').storage_audit(pathlib.Path(sys.argv[1]), {'generation':sys.argv[2]}, allow_stopped=sys.argv[3]=='1'))`,
    root,generation,allowStopped ? '1' : '0'], {env:{PATH:process.env.PATH},encoding:'utf8'});
  try {
    const ledger = Buffer.alloc(2*1048576);
    const terminal = record(1,2*1048576,4*1048576,true,2*1048576,8*1048576);
    terminal.copy(ledger,0); terminal.copy(ledger,256);
    fs.writeFileSync(path.join(root,'.storage-owner-000'),'');
    const target = path.join(root,'.storage-ledger');
    fs.writeFileSync(target,ledger);
    assert.equal(check().status,0,check().stderr);
    const revocation = ledger.length - 65536;
    ledger[revocation] = 1; fs.writeFileSync(target,ledger);
    assert.notEqual(check().status,0, 'revocation admitted complete storage evidence');
    const revoked = check(true);
    assert.equal(revoked.status,0,revoked.stderr);
    assert.match(revoked.stdout, /'stopped': True/);
    ledger[revocation + 1] = 1; fs.writeFileSync(target,ledger);
    assert.notEqual(check(true).status,0, 'unrelated tail corruption accepted');
    ledger[revocation + 1] = 0;
    ledger[revocation] = 0;
    // Intact latest anchor cannot hide a missing/torn issuance record.
    ledger[256+99] ^= 1; fs.writeFileSync(target,ledger);
    assert.notEqual(check().status,0);
    // Re-checking only hashes would miss an internally valid overspend.
    const overspend = record(2,2*1048576,1016*1048576+1,true,0,1012*1048576+1);
    terminal.copy(ledger,256); overspend.copy(ledger,0); overspend.copy(ledger,512);
    fs.writeFileSync(target,ledger);
    assert.notEqual(check().status,0);
  } finally { fs.rmSync(root,{recursive:true}); }
});
