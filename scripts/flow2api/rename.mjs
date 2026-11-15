import fs from 'node:fs';
import { setTimeout } from 'node:timers/promises';

// Windows antivirus can briefly hold newly downloaded runtime files open.
export async function renameWithRetry(source, destination) {
  for (let attempt = 0; ; attempt++) {
    try { fs.renameSync(source, destination); return; }
    catch (error) {
      if (!['EPERM', 'EBUSY', 'EACCES'].includes(error.code) || attempt >= 8) throw error;
      await setTimeout(Math.min(250 * 2 ** attempt, 2000));
    }
  }
}
