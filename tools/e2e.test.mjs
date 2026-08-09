import { afterAll, expect, test } from 'bun:test'
import { lstat, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { zipSync } from 'fflate'

import {
  cleanArtifacts,
  compareFileSets,
  compareSourceSnapshots,
  discoverModEntries,
  extractZipBytesToDirectory,
  loadConfig,
  materializeMods,
  parseConfig,
  patchOutcomes,
  resolveRecordedArtifact,
  saveConfig,
  snapshotDirectory,
} from './e2e.mjs'

const roots = []

async function testRoot() {
  const root = await mkdtemp(join(tmpdir(), 'hd2-repatcher-e2e-test-'))
  roots.push(root)
  return root
}

async function pathExists(path) {
  try {
    await lstat(path)
    return true
  } catch (error) {
    if (error?.code === 'ENOENT') return false
    throw error
  }
}

afterAll(async () => {
  for (const root of roots) await rm(root, { recursive: true, force: true })
})

test('config TOML round trips absolute game and relative source paths', async () => {
  const root = await testRoot()
  const configPath = join(root, 'r', 'e2e', 'config.toml')
  const config = {
    game: resolve(root, 'games', 'Helldivers 2'),
    files: ['mods/extracted', 'mods/archive.zip'],
  }

  await saveConfig(config, configPath)
  expect(parseConfig(await readFile(configPath, 'utf8'))).toEqual(config)
  expect(await loadConfig(configPath)).toEqual(config)
})

test('discovery includes immediate extracted folders and ZIP archives only', async () => {
  const root = await testRoot()
  const mods = join(root, 'mods')
  await mkdir(mods, { recursive: true })
  await mkdir(join(mods, 'folder'))
  await writeFile(join(mods, 'archive.ZIP'), 'not used by discovery')
  await writeFile(join(mods, 'notes.txt'), 'ignored')
  await mkdir(join(mods, 'nested'))
  await writeFile(join(mods, 'nested', 'inside.zip'), 'not immediate')

  const entries = await discoverModEntries(mods)
  expect(entries.map((path) => path.slice(mods.length + 1))).toEqual([
    'archive.ZIP',
    'folder',
    'nested',
  ])
})

test('ZIP extraction rejects traversal before writing any entry', async () => {
  const root = await testRoot()
  const destination = join(root, 'extracted')
  const archive = zipSync({
    'good/file.txt': new TextEncoder().encode('good'),
    '../escape.txt': new TextEncoder().encode('bad'),
  })

  await expect(extractZipBytesToDirectory(archive, destination)).rejects.toThrow(/escapes/i)
  expect(await pathExists(join(root, 'escape.txt'))).toBe(false)
  expect(await pathExists(destination)).toBe(false)

  await expect(
    extractZipBytesToDirectory(
      zipSync({ '..\\windows-escape.txt': new TextEncoder().encode('bad') }),
      destination
    )
  ).rejects.toThrow(/escapes/i)
  expect(await pathExists(join(root, 'windows-escape.txt'))).toBe(false)

  const safeDestination = join(root, 'safe')
  await extractZipBytesToDirectory(
    zipSync({ 'folder/file.txt': new TextEncoder().encode('safe') }),
    safeDestination
  )
  expect(await readFile(join(safeDestination, 'folder', 'file.txt'), 'utf8')).toBe('safe')
})

test('baseline and run artifact materialization keeps complete files and patch groups', async () => {
  const root = await testRoot()
  const sourceRoot = join(root, 'mods')
  const folder = join(sourceRoot, 'extracted')
  await mkdir(folder, { recursive: true })
  await writeFile(join(folder, 'unit.patch_0'), 'before')
  await writeFile(join(folder, 'unit.patch_0.stream'), 'stream')
  await writeFile(join(folder, 'readme.txt'), 'kept')
  await writeFile(
    join(sourceRoot, 'archive.zip'),
    zipSync({ 'audio.patch_1': new TextEncoder().encode('archive') })
  )
  const config = { game: resolve(root, 'game'), files: ['mods/extracted', 'mods/archive.zip'] }

  const baselineRoot = join(root, 'baseline')
  const baselineMods = await materializeMods(config, root, baselineRoot)
  expect(baselineMods.map((mod) => mod.name)).toEqual(['extracted', 'archive'])
  expect(baselineMods[0].groups[0].files).toEqual(['unit.patch_0', 'unit.patch_0.stream'])
  expect(await pathExists(join(baselineRoot, 'extracted', 'readme.txt'))).toBe(true)
  await writeFile(join(baselineRoot, 'extracted', 'unit.patch_0.gpu_resources'), 'generated')
  const after = await snapshotDirectory(join(baselineRoot, 'extracted'))
  expect(patchOutcomes(baselineMods[0], after, 0)[0].files).toEqual([
    'unit.patch_0',
    'unit.patch_0.gpu_resources',
    'unit.patch_0.stream',
  ])

  const runRoot = join(root, 'runs', 'run-1')
  await materializeMods(config, root, runRoot)
  expect(await pathExists(join(runRoot, 'archive', 'audio.patch_1'))).toBe(true)
})

test('file manifests detect added, removed, and byte/hash mismatches', async () => {
  const root = await testRoot()
  const baseline = join(root, 'baseline')
  const run = join(root, 'run')
  await mkdir(join(baseline, 'nested'), { recursive: true })
  await mkdir(join(run, 'nested'), { recursive: true })
  await writeFile(join(baseline, 'same.txt'), 'same')
  await writeFile(join(baseline, 'nested', 'changed.bin'), 'before')
  await writeFile(join(baseline, 'removed.txt'), 'removed')
  await writeFile(join(run, 'same.txt'), 'same')
  await writeFile(join(run, 'nested', 'changed.bin'), 'after')
  await writeFile(join(run, 'added.txt'), 'added')

  const comparison = compareFileSets(
    await snapshotDirectory(baseline),
    await snapshotDirectory(run)
  )
  expect(comparison.matches).toBe(false)
  expect(comparison.added).toEqual(['added.txt'])
  expect(comparison.removed).toEqual(['removed.txt'])
  expect(comparison.changed.map((item) => item.path)).toEqual(['nested/changed.bin'])
})

test('source fingerprints report changed, removed, and added entries', () => {
  const expected = [
    { path: 'mods/changed', type: 'directory', digest: 'before' },
    { path: 'mods/removed.zip', type: 'zip', digest: 'gone' },
  ]
  const actual = [
    { path: 'mods/changed', type: 'directory', digest: 'after' },
    { path: 'mods/added.zip', type: 'zip', digest: 'new' },
  ]
  expect(compareSourceSnapshots(expected, actual)).toEqual([
    'source added: mods/added.zip',
    'source changed: mods/changed',
    'source removed: mods/removed.zip',
  ])
})

test('clean removes runs and optional baseline without touching config or mods', async () => {
  const root = await testRoot()
  const e2eRoot = join(root, 'e2e')
  await mkdir(join(e2eRoot, 'runs', 'one'), { recursive: true })
  await mkdir(join(e2eRoot, 'baseline', 'mod'), { recursive: true })
  await mkdir(join(e2eRoot, 'mods'), { recursive: true })
  await writeFile(join(e2eRoot, 'config.toml'), 'game = "/game"\nfiles = []\n')
  await writeFile(join(e2eRoot, 'mods', 'keep.txt'), 'keep')

  await cleanArtifacts(e2eRoot)
  expect(await pathExists(join(e2eRoot, 'runs'))).toBe(false)
  expect(await pathExists(join(e2eRoot, 'baseline'))).toBe(true)
  expect(await pathExists(join(e2eRoot, 'config.toml'))).toBe(true)
  expect(await pathExists(join(e2eRoot, 'mods'))).toBe(true)

  await cleanArtifacts(e2eRoot, true)
  expect(await pathExists(join(e2eRoot, 'baseline'))).toBe(false)
  expect(await pathExists(join(e2eRoot, 'config.toml'))).toBe(true)
  expect(await pathExists(join(e2eRoot, 'mods'))).toBe(true)
})

test('install path validation accepts only recorded artifact folder shapes', async () => {
  const root = await testRoot()
  const e2eRoot = join(root, 'e2e')
  expect(resolveRecordedArtifact('baseline/mod', e2eRoot).artifactName).toBe('mod')
  expect(resolveRecordedArtifact('runs/20260809T000000Z/mod', e2eRoot).artifactName).toBe('mod')
  for (const path of ['', '../mod', 'baseline', 'baseline/mod/file', 'runs/only-two', '/tmp/mod']) {
    expect(() => resolveRecordedArtifact(path, e2eRoot)).toThrow()
  }
})

test('source folder symlinks are rejected before artifact copying', async () => {
  const root = await testRoot()
  const source = join(root, 'mods', 'linked')
  await mkdir(join(root, 'mods'), { recursive: true })
  await mkdir(join(root, 'outside'))
  await writeFile(join(root, 'outside', 'file.patch_0'), 'outside')
  try {
    await symlink(join(root, 'outside'), source)
  } catch {
    // Symlinks are unavailable on a few development filesystems; the other
    // extraction and path checks still cover the safety contract there.
    return
  }
  await expect(
    materializeMods(
      { game: resolve(root, 'game'), files: ['mods/linked'] },
      root,
      join(root, 'out')
    )
  ).rejects.toThrow(/symbolic link/i)
})
