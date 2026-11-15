import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { renameWithRetry } from './rename.mjs';

const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'de-story-rename-'));
const original = fs.renameSync;
try {
  const source = path.join(directory, 'staging');
  const destination = path.join(directory, 'published');
  fs.mkdirSync(source);
  fs.writeFileSync(path.join(source, 'manifest.json'), 'verified-runtime');
  let calls = 0;
  fs.renameSync = (...args) => {
    if (++calls === 1) throw Object.assign(new Error('Temporary scanner lock'), {code:'EPERM'});
    return original(...args);
  };
  await renameWithRetry(source, destination);
  assert.equal(calls, 2);
  assert.equal(fs.readFileSync(path.join(destination, 'manifest.json'), 'utf8'), 'verified-runtime');
  fs.renameSync = () => { throw Object.assign(new Error('Invalid path'), {code:'ENOENT'}); };
  await assert.rejects(renameWithRetry(source, destination), {code:'ENOENT'});
  console.log('Runtime publishing recovers from transient Windows locks and preserves permanent errors.');
} finally {
  fs.renameSync = original;
  fs.rmSync(directory, {recursive:true, force:true});
}
