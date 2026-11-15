/** Fit the complete frame above the playback controls, including narrow portrait previews. */
export function fitPreviewBox(width: number, height: number, ratio: number): { w: number; h: number } {
  const availableWidth = Math.max(1, width - 16)
  const availableHeight = Math.max(1, height - 16 - 48)
  const w = Math.max(1, Math.min(availableWidth, availableHeight * ratio))
  return { w, h: w / ratio }
}
