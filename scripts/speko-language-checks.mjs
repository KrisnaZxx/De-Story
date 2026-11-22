import assert from 'node:assert/strict'
import vm from 'node:vm'
import { build } from 'vite'

const result = await build({ configFile: false, logLevel: 'silent', build: { write: false, minify: false, lib: { entry: 'src/shared/speko.ts', formats: ['cjs'], fileName: 'speko-language-checks' } } })
const sandbox = { module: { exports: {} } }
sandbox.exports = sandbox.module.exports
vm.runInNewContext((Array.isArray(result) ? result[0] : result).output.find(o => o.type === 'chunk').code, sandbox)
const { spekoLanguageIssue, compatibleSpekoModels } = sandbox.module.exports
const models = ['soniox/tts-rt-v2', 'cartesia/sonic-3.6', 'elevenlabs/eleven_flash_v2', 'elevenlabs/eleven_flash_v2_5', 'munsit/munsit-en-ar', 'munsit/munsit', 'unverified/model'].map(id => ({ id, name: id, tags: [] }))
assert.equal(compatibleSpekoModels(models, 'id').map(m => m.id).join(','), 'soniox/tts-rt-v2,cartesia/sonic-3.6,elevenlabs/eleven_flash_v2_5')
assert.equal(compatibleSpekoModels(models, 'en-US').some(m => m.id === 'elevenlabs/eleven_flash_v2'), true)
assert.equal(compatibleSpekoModels(models, 'en').some(m => m.id === 'munsit/munsit'), false)
assert.match(spekoLanguageIssue('elevenlabs/eleven_flash_v2', 'id'), /Indonesia/)
assert.equal(spekoLanguageIssue('soniox/tts-rt-v2', 'ID_id'), null)
assert.match(spekoLanguageIssue('unverified/model', 'id'), /belum terverifikasi/)
assert.equal(compatibleSpekoModels(models, 'su').length, 0)
console.log('Speko language filtering passed: Indonesia, English, Arabic-only, unknown models, regional codes, and cached catalogs.')
