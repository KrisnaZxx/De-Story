import catalog from './speko-languages.json'
import { LANGUAGES } from './models'
import type { ModelOption } from './types'

export function spekoLanguages(id: string): string[] | null {
  return (catalog.models as Record<string, string[]>)[id] ?? null
}

export function spekoLanguageIssue(id: string, language: string): string | null {
  if (!id) return 'Pilih model Speko di Pengaturan terlebih dahulu.'
  const supported = spekoLanguages(id)
  if (!supported) return `Dukungan bahasa model ${id} belum terverifikasi. Pilih model Speko yang sudah terkonfirmasi.`
  const code = language.toLowerCase().replaceAll('_', '-').split('-')[0]
  const name = LANGUAGES.find(l => l.code === code)?.label ?? language
  return supported.some(l => l.toLowerCase().split('-')[0] === code) ? null : `Model ${id} tidak mendukung bahasa ${name}. Pilih model lain atau ubah bahasa proyek.`
}

export function compatibleSpekoModels(models: ModelOption[], language: string): ModelOption[] {
  return models.filter(m => !spekoLanguageIssue(m.id, language))
}
