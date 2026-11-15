import assert from 'node:assert/strict'
import { fitPreviewBox } from '../src/shared/preview.ts'

for (const [width, height] of [[340, 260], [600, 350], [900, 600]]) {
  for (const ratio of [9 / 16, 16 / 9]) {
    const box = fitPreviewBox(width, height, ratio)
    assert(box.w > 0 && box.h > 0)
    assert(box.w <= width - 16 + 0.01)
    assert(box.h <= height - 64 + 0.01)
    assert(Math.abs(box.w / box.h - ratio) < 0.0001)
  }
}
console.log('Portrait and landscape previews fit compact and large editor panels')
