import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'
import { build } from 'vite'

const result = await build({ configFile: false, logLevel: 'silent', build: { write: false, minify: false, lib: { entry: 'src/renderer/src/lib/keys.ts', formats: ['cjs'], fileName: 'speech-checks' } } })
const sandbox = { module: { exports: {} } }
sandbox.exports = sandbox.module.exports
vm.runInNewContext((Array.isArray(result) ? result[0] : result).output.find((o) => o.type === 'chunk').code, sandbox)
const { ttsReadiness, usableTts } = sandbox.module.exports
const keys = [{ provider: 'gemini', configured: true, lastOk: false }, { provider: 'speko', configured: true, lastOk: true }]
assert.equal(usableTts('gemini', keys), 'speko')
assert.equal(ttsReadiness(keys).elevenlabs.ok, false)
assert.equal(ttsReadiness(keys).gemini.ok, false)
assert.equal(ttsReadiness(keys).speko.ok, true)
assert.equal(usableTts('elevenlabs', []), 'elevenlabs')

const css = await readFile('src/renderer/src/styles.css', 'utf8')
const dark = css.slice(css.indexOf('@theme'), css.indexOf(':root'))
const light = css.slice(css.indexOf(":root[data-theme='light']"), css.indexOf('@font-face'))
const luminance = (hex) => {
  const values = hex.match(/\w\w/g).map((s) => parseInt(s, 16) / 255).map((v) => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4)
  return .2126 * values[0] + .7152 * values[1] + .0722 * values[2]
}
for (const palette of [dark, light]) {
  const colors = Object.fromEntries([...palette.matchAll(/--color-([\w-]+): #(\w{6});/g)].map((m) => [m[1], m[2]]))
  for (const [fg, bg] of [['ink', 'paper'], ['ink-2', 'surface'], ['muted', 'surface'], ['accent-ink', 'accent-soft'], ['sun-ink', 'sun-soft'], ['bad-ink', 'bad-soft'], ['ok-ink', 'ok-soft'], ['info-ink', 'info-soft']]) {
    const a = luminance(colors[fg]), b = luminance(colors[bg])
    assert.ok((Math.max(a,b)+.05)/(Math.min(a,b)+.05) >= 4.5, `${fg} on ${bg} must meet normal-text contrast`)
  }
  const accent = luminance(colors.accent)
  assert.ok(1.05/(accent+.05) >= 4.5, 'White primary-button text must remain readable')
}
console.log('Three-provider voice readiness and both theme palettes passed.')
