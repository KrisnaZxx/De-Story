import type { AppSettings } from '@shared/types'

export function applyTheme(theme: AppSettings['theme'] = 'dark'): void {
  const value = theme === 'light' ? 'light' : 'dark'
  document.documentElement.dataset.theme = value
  try { localStorage.setItem('de-story-theme', value) } catch { /* Storage may be unavailable during startup. */ }
}
