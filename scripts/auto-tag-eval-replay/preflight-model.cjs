// File-only preparation. No HTTP client, credential export or execution mode.
const fs = require('node:fs');
const { parseEnv } = require('node:util');
const { createHash } = require('node:crypto');
const sha = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const requireValue = (ok, code) => { if (!ok) throw Error(code); };
const modelId = value => typeof value === 'string' && /^~?deepseek\/[a-zA-Z0-9][a-zA-Z0-9._:/-]{0,140}$/.test(value);
const parameters = ['max_tokens', 'temperature', 'response_format', 'reasoning'];
const SCALE = 1000000000000n;

function rate(value) {
  requireValue(typeof value === 'string' && /^\d{1,6}(\.\d{1,12})?$/.test(value), 'preflight_price_unknown');
  const [whole, fraction = ''] = value.split('.');
  return BigInt(whole) * SCALE + BigInt(fraction.padEnd(12, '0'));
}
function decimal(value) {
  const fraction = String(value % SCALE).padStart(12, '0').replace(/0+$/, '');
  return String(value / SCALE) + (fraction ? '.' + fraction : '');
}

function preflight(envText, modelSnapshot, endpointSnapshot) {
  requireValue(typeof envText === 'string' && Buffer.byteLength(envText) <= 65536, 'preflight_config_invalid');
  // parseEnv does not execute shell substitutions or mutate process.env.
  const env = parseEnv(envText);
  const model = env.LLM_MODEL;
  requireValue(modelId(model), 'preflight_model_invalid');
  requireValue(typeof env.LLM_API_KEY === 'string' && env.LLM_API_KEY.trim().length > 0
    && !env.LLM_API_KEY.startsWith('****'), 'preflight_credential_missing');
  requireValue(!env.LLM_PROVIDER || env.LLM_PROVIDER === 'openrouter', 'preflight_provider_unsupported');
  const endpoint = env.LLM_BASE_URL || 'https://openrouter.ai/api/v1';
  requireValue(endpoint === 'https://openrouter.ai/api/v1', 'preflight_endpoint_unsupported');
  const m = modelSnapshot?.data, h = endpointSnapshot?.data;
  requireValue(m?.id === model && parameters.every(p => m.supported_parameters?.includes(p))
    && m.reasoning?.mandatory === false, 'preflight_model_catalog_mismatch');
  const target = m.alias_target?.slug || m.id;
  requireValue(modelId(target) && h?.id === target && Array.isArray(h.endpoints), 'preflight_target_mismatch');
  const candidates = [];
  for (const e of h.endpoints) {
    if (e.model_id !== target || e.status !== 0 || typeof e.tag !== 'string'
      || !/^[a-z0-9][a-z0-9_/-]{0,99}$/.test(e.tag) || e.tag.split('/').some(part => !part)
      || ![...parameters, 'structured_outputs'].every(p => e.supported_parameters?.includes(p))) continue;
    try {
      const pricing = e.pricing;
      const input = rate(pricing.prompt), output = rate(pricing.completion);
      if (pricing.input_cache_read !== undefined && rate(pricing.input_cache_read) > input) continue;
      // No assumption that non-text fees or cache writes are free.
      if (Object.keys(pricing).some(k => !['prompt', 'completion', 'discount', 'input_cache_read'].includes(k)
        && rate(pricing[k]) !== 0n)) continue;
      const estimate = 8192n * input + 1024n * output;
      if (estimate <= 5n * SCALE) candidates.push({ route: e.tag, estimate, pricing });
    } catch { /* Unknown pricing is ineligible, not a guessed zero. */ }
  }
  candidates.sort((a, b) => a.estimate < b.estimate ? -1 : a.estimate > b.estimate ? 1 : a.route.localeCompare(b.route));
  requireValue(candidates.length > 0, 'preflight_compatible_route_unavailable');
  const selected = candidates[0];
  const settings = { model, endpoint, provider: 'openrouter', source: 'env_local_only' };
  const report = {
    schemaVersion: 1, ...settings, observedTarget: target, credentialPresent: true,
    candidateRoute: selected.route, allowanceUsdCents: 500,
    settingsSha256: sha(settings), catalogSha256: sha({ modelSnapshot, endpointSnapshot }),
    limits: { modelAttempts: 1, retries: 0, repairs: 0, outputTokens: 1024,
      requestBytes: 32768, responseBytes: 65536, attemptMs: 5000,
      preparationMs: 300000, executionMs: 600000,
      growthBytes: 536870912, cumulativeWriteBytes: 1073741824,
      terminalGrowthBytes: 2097152, terminalWriteBytes: 8388608 },
    textEstimate: { usd: decimal(selected.estimate), inputTokens: 8192, outputTokens: 1024,
      pricing: { prompt: selected.pricing.prompt, completion: selected.pricing.completion },
      hardBillingCap: false,
      limitations: 'Declared text-token illustration, not a tokenizer proof or all-in fee/account guarantee.' },
    retention: { remote: 'documented_provider_defaults_for_public_metadata', local: 'until_result_review' },
    catalogFreshness: 'operator_supplied_snapshot_not_attested',
    liveAuthority: false, liveDispatchAvailable: false, readyForExecution: false,
    blockers: ['live_dispatch_disabled', 'real_run_approval_missing',
      'native_credential_isolation_not_integrated', 'selected_host_terms_not_bound',
      'billing_exposure_not_verified'],
  };
  return { ...report, planSha256: sha(report) };
}

if (require.main === module) {
  try {
    const args = process.argv.slice(2);
    requireValue(args.length === 3 && args.every(p => !p.startsWith('-')), 'preflight_arguments_invalid');
    let inputs;
    try {
      inputs = args.map(p => {
        const fd = fs.openSync(p, 'r');
        try {
          requireValue(fs.fstatSync(fd).isFile(), 'preflight_input_unavailable');
          const bytes = Buffer.alloc(1048577);
          let size = 0, count;
          while (size < bytes.length && (count = fs.readSync(fd, bytes, size, bytes.length - size, null)) > 0) size += count;
          requireValue(size <= 1048576, 'preflight_input_unavailable');
          return bytes.subarray(0, size).toString('utf8');
        } finally { fs.closeSync(fd); }
      });
      inputs = [inputs[0], JSON.parse(inputs[1]), JSON.parse(inputs[2])];
    } catch { throw Error('preflight_input_unavailable'); }
    process.stdout.write(JSON.stringify(preflight(...inputs), null, 2) + '\n');
  } catch (error) {
    const code = /^preflight_[a-z_]+$/.test(error.message) ? error.message : 'preflight_invalid';
    process.stderr.write(code + '\n');
    process.exitCode = 1;
  }
}
module.exports = { preflight };
