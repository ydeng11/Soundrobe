// Mock-only output bridge to the existing native storage boundary.
const fs = require('node:fs');
const path = require('node:path');
const { spawn, spawnSync } = require('node:child_process');
const { once } = require('node:events');
const entry = ['--exact', 'infra::mock_run_authority::storage::tests::storage_sink_process', '--ignored', '--nocapture'];
let logBytes = 0;

function scoped() {
  if (process.env.SOUNDROBE_NATIVE_MANAGED === '1' && process.env.SOUNDROBE_STORAGE_REQUIRED !== '1') {
    throw Error('managed output has no storage authority');
  }
  return process.env.SOUNDROBE_STORAGE_REQUIRED === '1';
}
function environment(target, action) {
  const keys = ['SOUNDROBE_NATIVE_OUTPUT', 'SOUNDROBE_NATIVE_GENERATION', 'SOUNDROBE_STORAGE_EXECUTABLE'];
  if (keys.some(key => !process.env[key])) throw Error('storage identity missing');
  return { ...Object.fromEntries(keys.map(key => [key, process.env[key]])),
    SOUNDROBE_STORAGE_REQUIRED: '1', SOUNDROBE_STORAGE_TARGET: path.resolve(target), SOUNDROBE_STORAGE_ACTION: action };
}
function sink(target, action, input = Buffer.alloc(0)) {
  if (input.length > 8388608) throw Error('storage helper input ceiling');
  const env = environment(target, action);
  const result = spawnSync(env.SOUNDROBE_STORAGE_EXECUTABLE, entry,
    { env, input, maxBuffer: 65536, stdio: ['pipe', action === 'quota_control' ? 'pipe' : 'ignore', 'ignore'] });
  if (action === 'quota_control') {
    // The native denial branch emits a bounded, nonblocking observer event.
    // Forward only that event; no run-owned write or ordinary admission may
    // follow the failing helper, and absence is not accepted as denial proof.
    for (const line of (result.stdout?.toString() ?? '').split('\n')) {
      if (line.startsWith('SOUNDROBE_STORAGE_DENIAL ')) process.stderr.write(line + '\n');
    }
    process.exit(78);
  }
  if (result.error || result.status !== 0 || result.signal) throw Error('native storage admission unavailable');
}
function writeFileSync(target, value, options) {
  if (!scoped()) return fs.writeFileSync(target, value, options);
  const encoding = typeof options === 'string' ? options : options?.encoding ?? 'utf8';
  if (encoding !== 'utf8' && encoding !== 'utf-8') throw Error('unsupported scoped output encoding');
  const flag = typeof options === 'object' ? options.flag ?? 'w' : 'w';
  if (!['w', 'wx'].includes(flag)) throw Error('unsupported scoped output flags');
  sink(target, flag === 'wx' ? 'create' : 'overwrite', Buffer.from(value));
}
function mkdirSync(target, options) {
  if (!scoped()) return fs.mkdirSync(target, options);
  sink(target, options?.recursive ? 'mkdir_all' : 'mkdir');
}
function copyFileSync(source, target, flags) {
  if (!scoped()) return fs.copyFileSync(source, target, flags);
  if (fs.statSync(source).size > 8388608) throw Error('scoped copy input ceiling');
  writeFileSync(target, fs.readFileSync(source), { flag: flags === fs.constants.COPYFILE_EXCL ? 'wx' : 'w' });
}
function openSync(target, flags, mode) {
  if (scoped() && !['r', 'rs'].includes(flags)) throw Error('unmetered writable descriptor denied');
  return fs.openSync(target, flags, mode);
}

function logSink(target) {
  if (!scoped()) throw Error('scoped log sink required');
  const env = environment(target, 'stream');
  const child = spawn(env.SOUNDROBE_STORAGE_EXECUTABLE, entry, { env, stdio: ['pipe', 'ignore', 'ignore'] });
  const settled = once(child, 'exit');
  let finishing;
  // The helper owns no native lease. Its failure must close the driver's
  // ownership immediately, even when the evaluator never emits another byte.
  child.once('exit', (code, signal) => {
    if (!finishing || code !== 0 || signal) process.exit(78);
  });
  child.once('error', () => process.exit(78));
  child.stdin.on('error', () => { process.exit(78); });
  return {
    attach(stream) {
      stream.on('data', chunk => {
        logBytes += chunk.length;
        if (logBytes > 2097152) {
          // Descriptor loss wakes the existing native containment owners.
          // No fallback log or damaged-export parsing may follow.
          process.exit(78);
        }
      });
      stream.pipe(child.stdin, { end: false });
    },
    finish() {
      return finishing ??= (async () => {
        child.stdin.end();
        const [code, signal] = await settled;
        if (code !== 0 || signal) throw Error('storage log settlement unavailable');
      })();
    },
  };
}

function writeLauncherOutcome(executable, root, generation, outcome) {
  const bytes = Buffer.from(JSON.stringify(outcome) + '\n');
  if (bytes.length > 65536) throw Error('launcher receipt ceiling');
  const result = spawnSync(executable, entry, { env: {
    SOUNDROBE_NATIVE_OUTPUT: root, SOUNDROBE_NATIVE_GENERATION: generation,
    SOUNDROBE_STORAGE_REQUIRED: '1', SOUNDROBE_STORAGE_TARGET: path.join(root, 'launcher-outcome.json'),
    SOUNDROBE_STORAGE_ACTION: 'terminal_launcher',
  }, input: bytes, stdio: ['pipe', 'ignore', 'ignore'] });
  if (result.error || result.status !== 0 || result.signal) throw Error('launcher storage unavailable');
}

const overrides = { writeFileSync, mkdirSync, copyFileSync, openSync, logSink, writeLauncherOutcome,
  quotaControl: target => sink(target, 'quota_control') };
const unsupported = /^(?:appendFile|write|writev|truncate|ftruncate|rename|link|symlink|unlink|rm|rmdir|mkdtemp|createWriteStream|cp|copyFile|mkdir|chmod|chown|utimes)/;
module.exports = new Proxy(fs, {
  get(target, key) {
    if (Object.hasOwn(overrides, key)) return overrides[key];
    if (scoped() && (key === 'promises' || unsupported.test(String(key)))) {
      throw Error('unsupported scoped filesystem effect');
    }
    return target[key];
  },
});
