import assert from 'node:assert/strict'
import { webcrypto } from 'node:crypto'
import { build } from 'vite'
import vm from 'node:vm'
import path from 'node:path'

async function moduleAt(entry) {
  const result = await build({ configFile: false, logLevel: 'silent', resolve: { alias: { '@shared': path.resolve('src/shared') } }, build: { write: false, minify: false, lib: { entry, formats: ['cjs'], fileName: 'checks' } } })
  const sandbox = { module: { exports: {} }, process: { env: { NODE_ENV: 'production' } }, crypto: webcrypto, setTimeout: () => 1, clearTimeout: () => {}, window: {} }
  sandbox.exports = sandbox.module.exports
  vm.runInNewContext((Array.isArray(result) ? result[0] : result).output.find(o => o.type === 'chunk').code, sandbox)
  return sandbox.module.exports
}
const { captionWords, effectiveDurationMs, editCaptionText, shiftCaptionRange } = await moduleAt('src/shared/editing.ts')
const original = [{ text: 'Halo', start: .4, end: .8 }, { text: 'dunia', start: 1, end: 1.5 }]
assert.equal(effectiveDurationMs(500, 8000), 8000)
assert.equal(captionWords({ captionOffsetSec: -.2 }, 'c', 'a', original, 2)[0].start, .2)
assert.equal(captionWords({ captionEdits: { c: { audioAssetId: 'old', words: [] } } }, 'c', 'new', original, 2).length, 2)
assert.equal(editCaptionText(original, 'Salam semua', 2)[1].start, 1)
assert.equal(editCaptionText(original, 'Satu dua tiga', 2).length, 3)
const shifted = shiftCaptionRange(original, 0, 1, 10, 2)
assert.equal(shifted[0].end, 1)
assert.equal(original[0].end, .8)
const { useProject } = await moduleAt('src/renderer/src/store/project.ts')
const store = () => useProject.getState()
store().load({ project: { id: 'p', step: 4, status: 'editing', editor: {} }, characters: [{ id: 'ch', sheetAssetId: 'sheet' }], clips: [{ id: 'c', projectId: 'p', durationMs: 5000, title: 'Old', audioAssetId: 'a', characterIds: ['ch'] }], assets: [], jobs: [] })
store().updateClip('c', { title: 'New' })
store().applyClip('c', { audioAssetId: 'generated-after-edit' })
store().undo()
assert.equal(store().clips[0].title, 'Old')
assert.equal(store().clips[0].audioAssetId, 'generated-after-edit')
store().redo()
assert.equal(store().clips[0].title, 'New')
store().removeClip('c'); assert.equal(store().clips.length, 0)
store().undo(); assert.equal(store().clips[0].audioAssetId, 'generated-after-edit')
store().removeCharacter('ch'); store().undo()
assert.equal(store().characters[0].sheetAssetId, 'sheet')
store().beginEdit()
for (let i = 0; i < 10; i++) store().updateEditor({ captionOffsetSec: i / 10 })
store().endEdit(); store().undo()
assert.equal(store().project.editor.captionOffsetSec, undefined)
store().load({ project: { id: 'other', editor: {} }, clips: [], characters: [], assets: [], jobs: [] })
assert.equal(store().canUndo, false)
console.log('Caption timing, narration duration, grouped undo/redo, restored rows, and generated-media preservation passed.')
