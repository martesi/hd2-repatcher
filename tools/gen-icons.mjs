#!/usr/bin/env node
// Regenerates src-tauri/icons/ from assets/icon-source.png via `tauri icon`.
// Icons are gitignored (generated, platform-specific, and noisy to diff), so
// this runs as a postinstall step and only acts when the icons dir is absent
// — it never clobbers icons a contributor is actively iterating on.

import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)))
const source = path.join(root, 'assets', 'icon-source.png')
const outDir = path.join(root, 'src-tauri', 'icons')
const marker = path.join(outDir, 'icon.png')

if (existsSync(marker)) {
  process.exit(0)
}

if (!existsSync(source)) {
  console.error(`gen-icons: source image missing at ${source}`)
  process.exit(1)
}

const tauriBin = path.join(
  root,
  'node_modules',
  '.bin',
  process.platform === 'win32' ? 'tauri.cmd' : 'tauri'
)

const result = spawnSync(tauriBin, ['icon', source, '-o', outDir], {
  stdio: 'inherit',
})

process.exit(result.status ?? 1)
