#!/usr/bin/env node
// Regenerates src-tauri/icons/ from assets/icon-source.png via `tauri icon`.
// Icons are gitignored (generated, platform-specific, and noisy to diff), so
// this runs as a postinstall step and only acts when the icons dir is absent
// — it never clobbers icons a contributor is actively iterating on.

import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
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

// Run the CLI's entrypoint with our own node rather than the node_modules/.bin
// shim: shim naming is package-manager- and platform-specific (bun writes .exe
// shims on Windows, not the .cmd npm produces, and Node refuses to spawn a .cmd
// without a shell anyway), whereas tauri.js is a plain node script everywhere.
let tauriEntry
try {
  tauriEntry = createRequire(import.meta.url).resolve('@tauri-apps/cli/tauri.js')
} catch (err) {
  console.error(`gen-icons: cannot resolve @tauri-apps/cli — ${err.message}`)
  process.exit(1)
}

const result = spawnSync(process.execPath, [tauriEntry, 'icon', source, '-o', outDir], {
  stdio: 'inherit',
})

if (result.error) {
  console.error(`gen-icons: failed to run tauri icon — ${result.error.message}`)
  process.exit(1)
}

if (result.signal) {
  console.error(`gen-icons: tauri icon killed by signal ${result.signal}`)
  process.exit(1)
}

process.exit(result.status ?? 1)
