import assert from 'node:assert/strict'
import { build } from 'vite'
import vm from 'node:vm'

const result = await build({ configFile: false, logLevel: 'silent', build: { write: false, minify: false, lib: { entry: 'src/shared/higgsfield.ts', formats: ['cjs'], fileName: 'model-checks' } } })
const sandbox = { module: { exports: {} } }
sandbox.exports = sandbox.module.exports
vm.runInNewContext((Array.isArray(result) ? result[0] : result).output.find((o) => o.type === 'chunk').code, sandbox)
const { getVideoModel, hfModelOptions, fitDuration } = sandbox.module.exports
for (const id of ['vidgen:gemini-omni-1.2-unlimited', 'vidgen:legacy-id']) {
  const model = getVideoModel(id)
  assert.equal(model.family, 'Vidgen')
  assert.equal(fitDuration(model, 8), 15)
}
assert.ok(hfModelOptions('video').some((m) => m.id === 'vidgen:gemini-omni-1.2-unlimited'))
assert.ok(hfModelOptions('image').some((m) => m.id === 'vidgen:gpt-image-2.6-sunburst'))
console.log('Vidgen video selection preserves its provider and duration; image choices remain separate.')
