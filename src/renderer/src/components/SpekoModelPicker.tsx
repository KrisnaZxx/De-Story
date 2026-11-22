import { useState, type ComponentProps } from 'react'
import { LANGUAGES } from '@shared/models'
import { compatibleSpekoModels, spekoLanguageIssue } from '@shared/speko'
import { useApp } from '../store/app'
import { useProject } from '../store/project'
import { ModelPicker } from './ModelPicker'
import { Select } from './Select'

export function SpekoModelPicker({ defaultLanguage = 'id', ...props }: ComponentProps<typeof ModelPicker> & { defaultLanguage?: string }) {
  const project = useProject(s => s.project)
  const route = useApp(s => s.route)
  const projectLanguage = route.name === 'settings' && route.back?.name === 'project' && route.back.id === project?.id ? project.language : undefined
  const [language, setLanguage] = useState(projectLanguage ?? defaultLanguage)
  const compatible = compatibleSpekoModels(props.models, language)
  const issue = props.value ? spekoLanguageIssue(props.value, language) : null
  return <div className="space-y-2">
    <Select label="Bahasa untuk daftar model Speko" size="sm" value={language} onChange={setLanguage} options={LANGUAGES.map(l => ({value:l.code,label:l.label}))} className="max-w-[260px]" />
    <ModelPicker {...props} models={compatible} footerNote="Hanya model dengan dukungan bahasa terkonfirmasi. Bahasa proyek tetap mengikuti pilihan di Ide cerita." />
    {!props.loading && issue && <p role="alert" className="text-sm text-bad-ink">{issue} Pilihan tersimpan tetap dipertahankan.</p>}
    {!props.loading && props.models.length > compatible.length && <p className="text-xs text-muted">{props.models.length - compatible.length} model disembunyikan karena tidak mendukung bahasa ini atau belum terverifikasi.</p>}
  </div>
}
