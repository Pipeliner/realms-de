// Execute the actual page with only browser/media/transport boundaries doubled.
const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const source = fs.readFileSync(process.argv[2], 'utf8').split('<script>')[1].split('</script>')[0];

async function run(times) {
  const posts = [];
  const timers = new Map();
  let nextId = 0;
  let now = 0;
  let cancelled = 0;
  let lateCallback;
  const track = {readyState: 'live', stop() { this.readyState = 'ended'; }};
  const share = {};
  const status = {};
  const video = {
    videoWidth: 10, videoHeight: 10,
    play: async () => {},
    requestVideoFrameCallback(callback) {
      const id = ++nextId;
      if (times.length) {
        const mediaTime = times.shift();
        setImmediate(() => callback(now, {mediaTime, presentedFrames: id}));
      } else {
        lateCallback = callback;
        setImmediate(() => {
          now = 30000;
          for (const [key, timer] of [...timers]) {
            timers.delete(key);
            timer();
          }
        });
      }
      return id;
    },
    cancelVideoFrameCallback() { cancelled++; },
  };
  const context = {
    document: {
      getElementById: id => ({share, status, video})[id],
      createElement: () => ({getContext: () => ({drawImage() {}}), toBlob: fn => fn('png')}),
    },
    navigator: {mediaDevices: {getDisplayMedia: async () => ({getTracks: () => [track]})}, userAgent: 'test'},
    performance: {now: () => now},
    setTimeout: fn => { const id = ++nextId; timers.set(id, fn); return id; },
    clearTimeout: id => timers.delete(id),
    fetch: async (path, options) => { posts.push([path, options.body]); return {ok: true}; },
  };
  vm.runInNewContext(source, context);
  await Promise.race([share.onclick(), new Promise((_, reject) => setTimeout(() => reject(Error('page did not finish')), 500))]);
  assert.equal(track.readyState, 'ended');
  const count = posts.length;
  if (lateCallback) {
    lateCallback(now, {mediaTime: 99, presentedFrames: 99});
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(posts.length, count, 'callback after timeout must not upload or publish');
  }
  return {posts, cancelled};
}

(async () => {
  const success = await run([1, 1, NaN, 2]);
  const result = JSON.parse(success.posts.find(([path]) => path === '/result')[1]);
  assert.deepEqual(result.frames.map(frame => frame.mediaTime), [1, 2]);
  assert.equal(success.posts.filter(([path]) => path.startsWith('/frame-')).length, 2);
  const timeout = await run([1, 1]);
  assert.equal(timeout.posts.some(([path]) => path === '/result'), false);
  assert.ok(timeout.posts.some(([path, body]) => path === '/error' && body.includes('frame delivery timeout')));
  assert.ok(timeout.cancelled > 0);
  const many = await run([1, ...Array(40).fill(1), 2]);
  assert.equal(JSON.parse(many.posts.find(([path]) => path === '/result')[1]).callbacks.length, 32);
})().catch(error => { console.error(error); process.exitCode = 1; });
