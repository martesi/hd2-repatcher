import { spawn, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createReadStream, constants as fsConstants } from 'node:fs'
import {
  access,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  open,
  readdir,
  readFile,
  rename,
  rm,
  writeFile,
} from 'node:fs/promises'
import { basename, dirname, extname, isAbsolute, join, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

import { unzipSync } from 'fflate'
import { parse as parseToml, stringify as stringifyToml } from 'smol-toml'

const TOOL_FILE = fileURLToPath(import.meta.url)
export const REPO_ROOT = resolve(dirname(TOOL_FILE), '..')
export const E2E_ROOT = join(REPO_ROOT, 'r', 'e2e')
export const CONFIG_PATH = join(E2E_ROOT, 'config.toml')
const RESULT_VERSION = 1
const PATCH_FILE_PATTERN = /\.patch(?:_\d+)?$/
const ZIP_EXTENSION = '.zip'
const SAMPLE_SIZE = 64 * 1024

export class E2eError extends Error {
  constructor(message, options = {}) {
    super(message, options)
    this.name = 'E2eError'
  }
}

function isWindowsAbsolute(value) {
  return /^[a-zA-Z]:[\\/]/.test(value) || value.startsWith('\\\\')
}

function isAbsolutePortable(value) {
  return isAbsolute(value) || isWindowsAbsolute(value)
}

function toPosix(value) {
  return value.split(sep).join('/')
}

function fromPosix(value) {
  return value.split('/').join(sep)
}

function isInside(root, candidate) {
  const rootPath = resolve(root)
  const candidatePath = resolve(candidate)
  const rest = relative(rootPath, candidatePath)
  return rest === '' || (rest !== '..' && !rest.startsWith(`..${sep}`) && !isAbsolute(rest))
}

function relativeTo(root, candidate) {
  const result = relative(resolve(root), resolve(candidate))
  return result ? toPosix(result) : '.'
}

function assertConfigRelativePath(value, label = 'path') {
  if (typeof value !== 'string' || value.length === 0) {
    throw new E2eError(`${label} must be a non-empty relative path`)
  }
  if (isAbsolutePortable(value)) {
    throw new E2eError(`${label} must be relative to r/e2e: ${value}`)
  }
  if (value.includes('\0')) {
    throw new E2eError(`${label} contains a NUL byte`)
  }
}

function assertSafeRelativePath(value, label = 'path') {
  assertConfigRelativePath(value, label)
  const parts = value.replaceAll('\\', '/').split('/')
  if (parts.some((part) => part === '..')) {
    throw new E2eError(`${label} escapes its root: ${value}`)
  }
  if (parts.some((part) => part === '')) {
    throw new E2eError(`${label} contains an empty path component: ${value}`)
  }
  return parts.join('/')
}

async function existingPath(path) {
  try {
    return await lstat(path)
  } catch (error) {
    if (error?.code === 'ENOENT') return null
    throw error
  }
}

async function requireDirectory(path, description) {
  const info = await existingPath(path)
  if (!info?.isDirectory() || info.isSymbolicLink()) {
    throw new E2eError(`${description} is not a directory: ${path}`)
  }
  return info
}

async function requireRegularFile(path, description) {
  const info = await existingPath(path)
  if (!info?.isFile()) {
    throw new E2eError(`${description} is not a regular file: ${path}`)
  }
  return info
}

async function ensureSafeDirectory(root, target) {
  if (!isInside(root, target)) throw new E2eError(`path escapes its root: ${target}`)
  await mkdir(root, { recursive: true })
  const rootInfo = await existingPath(root)
  if (!rootInfo?.isDirectory() || rootInfo.isSymbolicLink()) {
    throw new E2eError(`path is not a safe directory: ${root}`)
  }
  let current = resolve(root)
  const rest = relative(current, resolve(target))
  if (rest === '..' || rest.startsWith(`..${sep}`) || isAbsolute(rest)) {
    throw new E2eError(`path escapes its root: ${target}`)
  }
  const parts = rest ? rest.split(sep) : []
  for (const part of parts) {
    current = join(current, part)
    const info = await existingPath(current)
    if (!info) {
      await mkdir(current)
    } else if (!info.isDirectory() || info.isSymbolicLink()) {
      throw new E2eError(`path is not a safe directory: ${current}`)
    }
  }
}

async function sha256File(path) {
  const hash = createHash('sha256')
  const stream = createReadStream(path)
  for await (const chunk of stream) hash.update(chunk)
  return hash.digest('hex')
}

async function sampledFileHash(path, size) {
  const hash = createHash('sha256')
  const handle = await open(path, 'r')
  try {
    const firstLength = Math.min(size, SAMPLE_SIZE)
    const first = Buffer.alloc(firstLength)
    const firstRead = await handle.read(first, 0, firstLength, 0)
    hash.update(first.subarray(0, firstRead.bytesRead))
    if (size > SAMPLE_SIZE) {
      const last = Buffer.alloc(SAMPLE_SIZE)
      const lastRead = await handle.read(last, 0, SAMPLE_SIZE, size - SAMPLE_SIZE)
      hash.update(last.subarray(0, lastRead.bytesRead))
    }
  } finally {
    await handle.close()
  }
  return hash.digest('hex')
}

async function walkFiles(root, current = '') {
  const directory = current ? join(root, fromPosix(current)) : root
  const entries = (await readdir(directory, { withFileTypes: true })).sort((a, b) =>
    a.name.localeCompare(b.name)
  )
  const files = []
  for (const entry of entries) {
    const child = current ? `${current}/${entry.name}` : entry.name
    const childPath = join(root, fromPosix(child))
    if (entry.isSymbolicLink()) {
      throw new E2eError(`symbolic links are not supported in e2e artifacts: ${child}`)
    }
    if (entry.isDirectory()) {
      files.push(...(await walkFiles(root, child)))
    } else if (entry.isFile()) {
      const info = await lstat(childPath)
      files.push({
        path: child,
        size: info.size,
        sha256: await sha256File(childPath),
      })
    } else {
      throw new E2eError(`unsupported filesystem entry in e2e artifact: ${child}`)
    }
  }
  return files
}

function manifestDigest(files) {
  const hash = createHash('sha256')
  for (const file of files) {
    hash.update(`${file.path}\0${file.size}\0${file.sha256}\n`)
  }
  return hash.digest('hex')
}

export async function snapshotDirectory(root) {
  await requireDirectory(root, 'snapshot root')
  const files = await walkFiles(root)
  return { files, digest: manifestDigest(files) }
}

function manifestMap(files) {
  const result = new Map()
  for (const file of files ?? []) {
    if (!file || typeof file.path !== 'string') continue
    result.set(file.path, file)
  }
  return result
}

/**
 * Compares complete file manifests. The changed list deliberately includes
 * both hashes and sizes so a caller can explain the mismatch without reading
 * either artifact again.
 */
export function compareFileSets(expected, actual) {
  const expectedFiles = Array.isArray(expected) ? expected : (expected?.files ?? [])
  const actualFiles = Array.isArray(actual) ? actual : (actual?.files ?? [])
  const expectedMap = manifestMap(expectedFiles)
  const actualMap = manifestMap(actualFiles)
  const added = []
  const removed = []
  const changed = []

  for (const path of [...expectedMap.keys()].sort()) {
    if (!actualMap.has(path)) {
      removed.push(path)
      continue
    }
    const before = expectedMap.get(path)
    const after = actualMap.get(path)
    if (before.sha256 !== after.sha256 || before.size !== after.size) {
      changed.push({ path, expected: before, actual: after })
    }
  }
  for (const path of [...actualMap.keys()].sort()) {
    if (!expectedMap.has(path)) added.push(path)
  }
  return {
    matches: added.length === 0 && removed.length === 0 && changed.length === 0,
    added,
    removed,
    changed,
  }
}

function normaliseArchiveEntry(name) {
  if (typeof name !== 'string' || name.length === 0 || name.includes('\0')) {
    throw new E2eError('ZIP contains an empty or invalid entry name')
  }
  const portable = name.replaceAll('\\', '/')
  if (portable.startsWith('/') || portable.startsWith('//') || /^[a-zA-Z]:/.test(portable)) {
    throw new E2eError(`ZIP entry uses an absolute path: ${name}`)
  }
  const directory = portable.endsWith('/')
  const parts = portable.split('/')
  if (parts.some((part) => part === '..')) {
    throw new E2eError(`ZIP entry escapes its extraction directory: ${name}`)
  }
  const cleanParts = parts.filter((part) => part.length > 0 && part !== '.')
  if (cleanParts.length === 0) {
    throw new E2eError(`ZIP entry has no usable path: ${name}`)
  }
  return { relative: cleanParts.join('/'), directory }
}

async function extractEntries(entries, destination) {
  const prepared = []
  const kinds = new Map()
  for (const name of Object.keys(entries)) {
    const normalised = normaliseArchiveEntry(name)
    const previous = kinds.get(normalised.relative)
    const kind = normalised.directory ? 'directory' : 'file'
    if (previous && previous !== kind) {
      throw new E2eError(`ZIP contains both a file and directory named ${normalised.relative}`)
    }
    if (previous) throw new E2eError(`ZIP contains duplicate entry ${normalised.relative}`)
    kinds.set(normalised.relative, kind)
    prepared.push({ ...normalised, bytes: entries[name] })
  }
  prepared.sort((a, b) => a.relative.localeCompare(b.relative))

  for (const entry of prepared) {
    const parts = entry.relative.split('/')
    for (let index = 1; index < parts.length; index += 1) {
      const ancestor = parts.slice(0, index).join('/')
      if (kinds.get(ancestor) === 'file') {
        throw new E2eError(`ZIP file ${ancestor} conflicts with entry ${entry.relative}`)
      }
    }
  }

  await ensureSafeDirectory(destination, destination)
  for (const entry of prepared) {
    const target = resolve(destination, fromPosix(entry.relative))
    if (!isInside(destination, target)) {
      throw new E2eError(`ZIP entry escapes its extraction directory: ${entry.relative}`)
    }
    if (entry.directory) {
      await ensureSafeDirectory(destination, target)
      continue
    }
    await ensureSafeDirectory(destination, dirname(target))
    const existingTarget = await existingPath(target)
    if (existingTarget && (!existingTarget.isFile() || existingTarget.isSymbolicLink())) {
      throw new E2eError(`ZIP file conflicts with an existing path: ${entry.relative}`)
    }
    await writeFile(target, entry.bytes)
  }
}

export async function extractZipBytesToDirectory(bytes, destination) {
  let entries
  try {
    entries = unzipSync(bytes)
  } catch (error) {
    throw new E2eError(`failed to read ZIP archive: ${error.message}`)
  }
  await extractEntries(entries, destination)
}

export async function extractZipToDirectory(archivePath, destination) {
  await requireRegularFile(archivePath, 'ZIP archive')
  const bytes = await readFile(archivePath)
  await extractZipBytesToDirectory(bytes, destination)
}

async function copyTreeEntry(source, destination) {
  const info = await lstat(source)
  if (info.isSymbolicLink()) {
    throw new E2eError(`symbolic links are not supported in mod sources: ${source}`)
  }
  if (info.isDirectory()) {
    await mkdir(destination, { recursive: true })
    const entries = (await readdir(source, { withFileTypes: true })).sort((a, b) =>
      a.name.localeCompare(b.name)
    )
    for (const entry of entries) {
      await copyTreeEntry(join(source, entry.name), join(destination, entry.name))
    }
  } else if (info.isFile()) {
    await mkdir(dirname(destination), { recursive: true })
    await copyFile(source, destination)
  } else {
    throw new E2eError(`unsupported filesystem entry in mod source: ${source}`)
  }
}

async function copyDirectoryContents(source, destination) {
  await requireDirectory(source, 'mod source')
  await mkdir(destination, { recursive: true })
  const entries = (await readdir(source, { withFileTypes: true })).sort((a, b) =>
    a.name.localeCompare(b.name)
  )
  for (const entry of entries) {
    await copyTreeEntry(join(source, entry.name), join(destination, entry.name))
  }
}

function isZipPath(path) {
  return extname(path).toLowerCase() === ZIP_EXTENSION
}

async function sourceType(sourcePath) {
  const info = await existingPath(sourcePath)
  if (!info || info.isSymbolicLink()) {
    throw new E2eError(`configured mod source does not exist or is a symbolic link: ${sourcePath}`)
  }
  if (info.isDirectory()) return 'directory'
  if (info.isFile() && isZipPath(sourcePath)) return 'zip'
  throw new E2eError(`configured mod source must be a folder or .zip archive: ${sourcePath}`)
}

async function snapshotSource(sourcePath, relativePath) {
  const type = await sourceType(sourcePath)
  if (type === 'zip') {
    const info = await lstat(sourcePath)
    return {
      path: relativePath,
      type,
      digest: await sha256File(sourcePath),
      file_count: 1,
      size: info.size,
    }
  }
  const snapshot = await snapshotDirectory(sourcePath)
  return {
    path: relativePath,
    type,
    digest: snapshot.digest,
    file_count: snapshot.files.length,
    size: snapshot.files.reduce((total, file) => total + file.size, 0),
  }
}

export async function discoverModEntries(sourceDirectory) {
  await requireDirectory(sourceDirectory, 'mod source directory')
  const entries = await readdir(sourceDirectory, { withFileTypes: true })
  return entries
    .filter((entry) => entry.isDirectory() || (entry.isFile() && isZipPath(entry.name)))
    .map((entry) => join(sourceDirectory, entry.name))
    .sort((a, b) => a.localeCompare(b))
}

export function parseConfig(text) {
  let parsed
  try {
    parsed = parseToml(text)
  } catch (error) {
    throw new E2eError(`failed to parse e2e config.toml: ${error.message}`)
  }
  if (!parsed || typeof parsed !== 'object') {
    throw new E2eError('e2e config.toml must contain a table')
  }
  if (typeof parsed.game !== 'string' || !isAbsolutePortable(parsed.game)) {
    throw new E2eError('e2e config.toml `game` must be an absolute install path')
  }
  if (!Array.isArray(parsed.files) || parsed.files.some((value) => typeof value !== 'string')) {
    throw new E2eError('e2e config.toml `files` must be an array of relative paths')
  }
  const files = parsed.files.map((value, index) => {
    assertConfigRelativePath(value, `files[${index}]`)
    return toPosix(value)
  })
  if (new Set(files).size !== files.length) {
    throw new E2eError('e2e config.toml `files` contains duplicate paths')
  }
  return { game: parsed.game, files }
}

export async function loadConfig(configPath = CONFIG_PATH) {
  try {
    return parseConfig(await readFile(configPath, 'utf8'))
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new E2eError(`missing ${configPath}; run \`bun run test:e2e:init\` first`)
    }
    throw error
  }
}

export async function saveConfig(config, configPath = CONFIG_PATH) {
  const normalised = parseConfig(stringifyToml(config))
  await mkdir(dirname(configPath), { recursive: true })
  await writeFile(configPath, `${stringifyToml(normalised)}\n`)
  return normalised
}

function resolveConfigSource(e2eRoot, configPath) {
  assertConfigRelativePath(configPath, 'configured mod source')
  return resolve(e2eRoot, fromPosix(configPath))
}

async function validateGameRoot(gameRoot) {
  const rootInfo = await existingPath(gameRoot)
  const dataPath = join(gameRoot, 'data')
  const dataInfo = await existingPath(dataPath)
  if (!rootInfo?.isDirectory() || rootInfo.isSymbolicLink() || !dataInfo?.isDirectory()) {
    throw new E2eError(
      `game install path is invalid: ${gameRoot} (expected ${join(gameRoot, 'data')})`
    )
  }
  const legacy = await existingPath(join(dataPath, '9ba626afa44a3aa3'))
  const slim = await existingPath(join(dataPath, 'bundles.nxa'))
  if (!legacy && !slim) {
    throw new E2eError(
      `game install path is invalid: ${gameRoot} (data must contain 9ba626afa44a3aa3 or bundles.nxa)`
    )
  }
  return dataPath
}

async function fingerprintGame(gameRoot) {
  const dataPath = await validateGameRoot(gameRoot)
  const files = await walkGameFiles(dataPath)
  const hash = createHash('sha256')
  for (const file of files) {
    hash.update(`${file.path}\0${file.size}\0${file.sample_sha256}\n`)
  }
  return {
    digest: hash.digest('hex'),
    file_count: files.length,
    markers: files
      .map((file) => file.path)
      .filter((path) => path === '9ba626afa44a3aa3' || path === 'bundles.nxa'),
  }
}

async function walkGameFiles(root, current = '') {
  const directory = current ? join(root, fromPosix(current)) : root
  const entries = (await readdir(directory, { withFileTypes: true })).sort((a, b) =>
    a.name.localeCompare(b.name)
  )
  const files = []
  for (const entry of entries) {
    const child = current ? `${current}/${entry.name}` : entry.name
    const childPath = join(root, fromPosix(child))
    if (entry.isSymbolicLink()) {
      throw new E2eError(`symbolic links are not supported in the game fingerprint: ${child}`)
    }
    if (entry.isDirectory()) {
      files.push(...(await walkGameFiles(root, child)))
    } else if (entry.isFile() && !isPatchArtifact(child)) {
      const info = await lstat(childPath)
      files.push({
        path: child,
        size: info.size,
        sample_sha256: await sampledFileHash(childPath, info.size),
      })
    }
  }
  return files
}

async function sourceSnapshots(config, e2eRoot) {
  return Promise.all(
    config.files.map(async (path) => {
      const sourcePath = resolveConfigSource(e2eRoot, path)
      return snapshotSource(sourcePath, path)
    })
  )
}

export function compareSourceSnapshots(expected, actual) {
  const expectedMap = new Map((expected ?? []).map((source) => [source.path, source]))
  const actualMap = new Map((actual ?? []).map((source) => [source.path, source]))
  const differences = []
  for (const [path, source] of expectedMap) {
    const current = actualMap.get(path)
    if (!current) {
      differences.push(`source removed: ${path}`)
    } else if (source.type !== current.type || source.digest !== current.digest) {
      differences.push(`source changed: ${path}`)
    }
  }
  for (const path of actualMap.keys()) {
    if (!expectedMap.has(path)) differences.push(`source added: ${path}`)
  }
  return differences.sort()
}

function artifactNameForSource(sourcePath) {
  const name = basename(sourcePath)
  return isZipPath(name) ? name.slice(0, -ZIP_EXTENSION.length) : name
}

function patchMain(path) {
  return PATCH_FILE_PATTERN.test(path)
}

function isPatchArtifact(path) {
  const name = basename(path)
  const main = name.replace(/\.(?:stream|gpu_resources)$/, '')
  return patchMain(main)
}

export function discoverPatchGroups(files) {
  const paths = new Set(files.map((file) => file.path))
  return files
    .filter((file) => patchMain(file.path))
    .map((file) => {
      const groupFiles = [file.path]
      for (const suffix of ['.stream', '.gpu_resources']) {
        if (paths.has(`${file.path}${suffix}`)) groupFiles.push(`${file.path}${suffix}`)
      }
      return { main: file.path, files: groupFiles }
    })
    .sort((a, b) => a.main.localeCompare(b.main))
}

export async function materializeMods(config, e2eRoot, artifactRoot) {
  await ensureSafeDirectory(artifactRoot, artifactRoot)
  const usedNames = new Set()
  const mods = []
  for (const source of config.files) {
    const sourcePath = resolveConfigSource(e2eRoot, source)
    const name = artifactNameForSource(sourcePath)
    if (!name || name === '.' || name === '..') {
      throw new E2eError(`could not derive an artifact folder name from ${source}`)
    }
    if (usedNames.has(name)) {
      throw new E2eError(`mod sources collide on artifact folder ${name}; rename one source`)
    }
    usedNames.add(name)
    const artifactPath = join(artifactRoot, name)
    await mkdir(artifactPath)
    const type = await sourceType(sourcePath)
    if (type === 'zip') await extractZipToDirectory(sourcePath, artifactPath)
    else await copyDirectoryContents(sourcePath, artifactPath)
    const before = await snapshotDirectory(artifactPath)
    mods.push({
      name,
      source,
      type,
      artifactPath,
      before,
      groups: discoverPatchGroups(before.files),
    })
  }
  return mods
}

function sameManifestEntry(before, after) {
  return Boolean(before && after && before.sha256 === after.sha256 && before.size === after.size)
}

export function patchOutcomes(mod, after, exitStatus) {
  const beforeMap = manifestMap(mod.before.files)
  const afterMap = manifestMap(after.files)
  const groups = new Map(mod.groups.map((group) => [group.main, new Set(group.files)]))
  for (const group of discoverPatchGroups(after.files)) {
    const files = groups.get(group.main) ?? new Set()
    for (const path of group.files) files.add(path)
    groups.set(group.main, files)
  }
  return [...groups.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([main, paths]) => {
      const groupFiles = [...paths].sort()
      const changedFiles = groupFiles.filter(
        (path) => !sameManifestEntry(beforeMap.get(path), afterMap.get(path))
      )
      let outcome = 'unchanged'
      if (changedFiles.length > 0) outcome = 'updated'
      else if (exitStatus !== 0) outcome = 'failed'
      return { main, files: groupFiles, changed_files: changedFiles, outcome }
    })
}

function describeTool(tool) {
  if (!tool) return 'not-found'
  return [tool.command, ...tool.prefix].join(' ')
}

function commandWorks(command) {
  const result = spawnSync(command, ['--version'], { stdio: 'ignore', windowsHide: true })
  return result.status === 0
}

async function isUsableFile(path) {
  const info = await existingPath(path)
  if (!info?.isFile()) return false
  if (process.platform === 'win32') return true
  try {
    await access(path, fsConstants.X_OK)
    return true
  } catch {
    return false
  }
}

export async function resolveTool(repoRoot = REPO_ROOT) {
  const configured = process.env.HD2_REPATCHER_BIN
  if (configured) {
    const path = resolve(process.cwd(), configured)
    if (!(await isUsableFile(path))) {
      throw new E2eError(`HD2_REPATCHER_BIN is not an executable file: ${path}`)
    }
    return { kind: 'binary', command: path, prefix: [] }
  }

  const releaseCandidates = [
    join(repoRoot, 'target', 'release', 'hd2-repatcher'),
    join(repoRoot, 'target', 'release', 'hd2-repatcher.exe'),
    join(repoRoot, 'src-tauri', 'target', 'release', 'hd2-repatcher'),
    join(repoRoot, 'src-tauri', 'target', 'release', 'hd2-repatcher.exe'),
  ]
  for (const candidate of releaseCandidates) {
    if (await isUsableFile(candidate)) {
      return { kind: 'binary', command: candidate, prefix: [] }
    }
  }

  if (commandWorks('cargo')) {
    return {
      kind: 'cargo',
      command: 'cargo',
      prefix: ['run', '--release', '--bin', 'hd2-repatcher', '--'],
    }
  }
  if (commandWorks('nix')) {
    return {
      kind: 'nix-cargo',
      command: 'nix',
      prefix: [
        'develop',
        '.',
        '--command',
        'cargo',
        'run',
        '--release',
        '--bin',
        'hd2-repatcher',
        '--',
      ],
    }
  }
  throw new E2eError(
    'could not find a repatcher executable; set HD2_REPATCHER_BIN, build target/release/hd2-repatcher, or install cargo/Nix'
  )
}

function runProcess(command, args, cwd) {
  return new Promise((resolveProcess) => {
    const child = spawn(command, args, {
      cwd,
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
    })
    let stdout = ''
    let stderr = ''
    let spawnError = null
    child.stdout?.setEncoding('utf8')
    child.stderr?.setEncoding('utf8')
    child.stdout?.on('data', (chunk) => {
      stdout += chunk
    })
    child.stderr?.on('data', (chunk) => {
      stderr += chunk
    })
    child.once('error', (error) => {
      spawnError = error
    })
    child.once('close', (code, signal) => {
      resolveProcess({
        exit_status: typeof code === 'number' ? code : 127,
        signal: signal ?? '',
        stdout,
        stderr,
        spawn_error: spawnError?.message ?? '',
      })
    })
  })
}

async function toolRevision(repoRoot) {
  const result = await runProcess('git', ['describe', '--always', '--dirty'], repoRoot)
  const revision = result.stdout.trim()
  return result.exit_status === 0 && revision ? revision : 'unknown'
}

function resultBase(kind, config, context, createdAt) {
  return {
    version: RESULT_VERSION,
    kind,
    created_at: createdAt,
    game_path: config.game,
    game_fingerprint: context.gameFingerprint,
    tool_revision: context.toolRevision,
    tool_command: 'not-run',
    exit_status: -1,
    stdout: '',
    stderr: '',
    errors: [],
    sources: context.sources,
    mods: [],
  }
}

async function executeScenario(kind, outputRoot, config, context, createdAt) {
  const result = resultBase(kind, config, context, createdAt)
  let materialized = []
  let processResult = null
  try {
    materialized = await materializeMods(config, context.e2eRoot, outputRoot)
    const tool = await resolveTool(context.repoRoot)
    result.tool_command = describeTool(tool)
    const patchPaths = materialized.map((mod) => mod.artifactPath)
    if (patchPaths.length > 0) {
      processResult = await runProcess(
        tool.command,
        [...tool.prefix, '--game', config.game, '--no-game-path-caching', ...patchPaths],
        context.repoRoot
      )
      result.exit_status = processResult.exit_status
      result.stdout = processResult.stdout
      result.stderr = processResult.stderr
      if (processResult.spawn_error) {
        result.errors.push(`failed to start repatcher: ${processResult.spawn_error}`)
      }
      if (processResult.exit_status !== 0) {
        result.errors.push(`repatcher exited with status ${processResult.exit_status}`)
      }
    } else {
      result.exit_status = 0
    }
  } catch (error) {
    result.errors.push(error instanceof Error ? error.message : String(error))
  }

  const exitStatus = processResult?.exit_status ?? result.exit_status
  for (const mod of materialized) {
    try {
      const after = await snapshotDirectory(mod.artifactPath)
      result.mods.push({
        name: mod.name,
        source: mod.source,
        artifact: mod.name,
        files: after.files,
        patch_groups: patchOutcomes(mod, after, exitStatus),
      })
    } catch (error) {
      result.errors.push(
        `failed to snapshot ${mod.name}: ${error instanceof Error ? error.message : String(error)}`
      )
    }
  }
  return result
}

async function writeResult(result, outputRoot) {
  await mkdir(outputRoot, { recursive: true })
  await writeFile(join(outputRoot, 'result.toml'), `${stringifyToml(result)}\n`)
}

async function loadResult(resultPath) {
  await requireRegularFile(resultPath, 'e2e result')
  let parsed
  try {
    parsed = parseToml(await readFile(resultPath, 'utf8'))
  } catch (error) {
    if (error?.code === 'ENOENT') throw new E2eError(`missing result.toml: ${resultPath}`)
    throw new E2eError(`failed to parse ${resultPath}: ${error.message}`)
  }
  if (!parsed || parsed.version !== RESULT_VERSION || !Array.isArray(parsed.mods)) {
    throw new E2eError(`invalid e2e result file: ${resultPath}`)
  }
  return parsed
}

function compareResults(baseline, current) {
  const expectedMods = new Map((baseline.mods ?? []).map((mod) => [mod.name, mod]))
  const actualMods = new Map((current.mods ?? []).map((mod) => [mod.name, mod]))
  const differences = []
  for (const [name, expected] of expectedMods) {
    const actual = actualMods.get(name)
    if (!actual) {
      differences.push(`${name}: artifact missing from run`)
      continue
    }
    const comparison = compareFileSets(expected.files, actual.files)
    for (const path of comparison.added) differences.push(`${name}: added ${path}`)
    for (const path of comparison.removed) differences.push(`${name}: removed ${path}`)
    for (const item of comparison.changed) {
      differences.push(
        `${name}: hash mismatch for ${item.path} (expected ${item.expected.sha256}, got ${item.actual.sha256})`
      )
    }
  }
  for (const name of actualMods.keys()) {
    if (!expectedMods.has(name)) differences.push(`${name}: unexpected artifact in run`)
  }
  return { matches: differences.length === 0, differences: differences.sort() }
}

async function verifyRecordedArtifacts(result, artifactRoot) {
  const differences = []
  for (const mod of result.mods ?? []) {
    const artifactPath = join(artifactRoot, mod.name)
    try {
      const actual = await snapshotDirectory(artifactPath)
      const comparison = compareFileSets(mod.files, actual.files)
      for (const path of comparison.added) differences.push(`${mod.name}: added ${path}`)
      for (const path of comparison.removed) differences.push(`${mod.name}: removed ${path}`)
      for (const item of comparison.changed) {
        differences.push(
          `${mod.name}: hash mismatch for ${item.path} (expected ${item.expected.sha256}, got ${item.actual.sha256})`
        )
      }
    } catch (error) {
      differences.push(
        `${mod.name}: could not verify recorded artifact (${error instanceof Error ? error.message : String(error)})`
      )
    }
  }
  return differences.sort()
}

async function executionContext(config) {
  const gameFingerprint = await fingerprintGame(config.game)
  const sources = await sourceSnapshots(config, E2E_ROOT)
  return {
    repoRoot: REPO_ROOT,
    e2eRoot: E2E_ROOT,
    gameFingerprint,
    sources,
    toolRevision: await toolRevision(REPO_ROOT),
  }
}

async function uniqueRunDirectory() {
  const runsRoot = join(E2E_ROOT, 'runs')
  await ensureSafeDirectory(E2E_ROOT, runsRoot)
  const now = new Date()
  const timestamp = now.toISOString().replaceAll(/[-:]/g, '').replace('.', '-')
  for (let index = 0; index < 1000; index += 1) {
    const suffix = index === 0 ? '' : `-${index}`
    const name = `${timestamp}${suffix}`
    const path = join(runsRoot, name)
    try {
      await mkdir(path)
      return { name, path }
    } catch (error) {
      if (error?.code !== 'EEXIST') throw error
    }
  }
  throw new E2eError('could not create a unique e2e run directory')
}

async function initCommand(options) {
  const current = await existingPath(CONFIG_PATH)
  let previous = null
  if (current) previous = await loadConfig(CONFIG_PATH)
  if (!options.game && !previous) {
    throw new E2eError('init needs --game PATH the first time it is run')
  }
  const game = options.game ? resolve(process.cwd(), options.game) : previous.game
  const sourceDirectory = options.mods
    ? resolve(process.cwd(), options.mods)
    : join(E2E_ROOT, 'mods')
  if (!options.mods && !(await existingPath(sourceDirectory))) {
    await mkdir(sourceDirectory, { recursive: true })
  }
  const entries = await discoverModEntries(sourceDirectory)
  const files = entries.map((entry) => relativeTo(E2E_ROOT, entry)).sort()
  const config = await saveConfig({ game, files }, CONFIG_PATH)
  console.log(`Initialized ${CONFIG_PATH} with ${config.files.length} mod source(s).`)
  if (config.files.length === 0) {
    console.log(`No mod folders or ZIP archives were found in ${sourceDirectory}.`)
  }
}

async function requireExecutionConfig() {
  const config = await loadConfig(CONFIG_PATH)
  if (config.files.length === 0) {
    throw new E2eError('e2e config has an empty `files` list; run init with a mod source directory')
  }
  await validateGameRoot(config.game)
  return config
}

async function baselineCommand(force) {
  const config = await requireExecutionConfig()
  const baselinePath = join(E2E_ROOT, 'baseline')
  const baselineInfo = await existingPath(baselinePath)
  if (baselineInfo && !force) {
    throw new E2eError(`baseline already exists at ${baselinePath}; pass --force to replace it`)
  }
  const context = await executionContext(config)
  const staging = await mkdtemp(join(E2E_ROOT, '.baseline-staging-'))
  let committed = false
  let result
  try {
    result = await executeScenario('baseline', staging, config, context, new Date().toISOString())
    await writeResult(result, staging)
    if (baselineInfo) await removeArtifactDirectory(baselinePath)
    await rename(staging, baselinePath)
    committed = true
  } finally {
    if (!committed) await rm(staging, { recursive: true, force: true })
  }
  if (result.errors.length > 0 || result.exit_status !== 0) {
    throw new E2eError(
      `baseline generation failed; inspect ${join(baselinePath, 'result.toml')} and rerun with --force after fixing the problem`
    )
  }
  console.log(`Baseline written to ${baselinePath}.`)
}

function failureResult(kind, config, context, createdAt, errors) {
  const result = resultBase(kind, config, context, createdAt)
  result.errors.push(...errors)
  result.comparison = { status: 'not-run', matches: false, differences: [] }
  return result
}

async function testCommand() {
  const config = await requireExecutionConfig()
  const baselinePath = join(E2E_ROOT, 'baseline')
  await requireDirectory(baselinePath, 'baseline')
  const baseline = await loadResult(join(baselinePath, 'result.toml'))
  if (baseline.kind !== 'baseline') throw new E2eError(`${baselinePath} is not a baseline result`)
  if (baseline.exit_status !== 0 || (baseline.errors?.length ?? 0) > 0) {
    throw new E2eError(`baseline is not successful; regenerate it with \`--force\``)
  }
  if (baseline.game_path !== config.game) {
    throw new E2eError('configured game path differs from the baseline; regenerate the baseline')
  }

  const run = await uniqueRunDirectory()
  let result
  try {
    const context = await executionContext(config)
    const sourceDifferences = compareSourceSnapshots(baseline.sources, context.sources)
    const preflightErrors = []
    const baselineArtifactDifferences = await verifyRecordedArtifacts(baseline, baselinePath)
    if (baselineArtifactDifferences.length > 0) {
      preflightErrors.push(
        'baseline artifact differs from its recorded hashes; regenerate it with --force'
      )
      preflightErrors.push(...baselineArtifactDifferences)
    }
    if (sourceDifferences.length > 0) {
      preflightErrors.push(...sourceDifferences)
    }
    if (baseline.game_fingerprint?.digest !== context.gameFingerprint.digest) {
      preflightErrors.push('game fingerprint differs from the baseline; regenerate the baseline')
    }
    if (baseline.tool_revision !== context.toolRevision) {
      preflightErrors.push('tool revision differs from the baseline; regenerate the baseline')
    }
    if (preflightErrors.length > 0) {
      result = failureResult('run', config, context, new Date().toISOString(), preflightErrors)
    } else {
      result = await executeScenario('run', run.path, config, context, new Date().toISOString())
      const comparison = compareResults(baseline, result)
      result.comparison = {
        status: comparison.matches ? 'matched' : 'different',
        matches: comparison.matches,
        differences: comparison.differences,
      }
      if (!comparison.matches) {
        result.errors.push('run output differs from the baseline')
      }
    }
  } catch (error) {
    const context = {
      e2eRoot: E2E_ROOT,
      repoRoot: REPO_ROOT,
      gameFingerprint: { digest: 'not-computed', file_count: 0, markers: [] },
      sources: [],
      toolRevision: 'not-computed',
    }
    result = failureResult('run', config, context, new Date().toISOString(), [
      error instanceof Error ? error.message : String(error),
    ])
  }
  await writeResult(result, run.path)
  const runRelative = relativeTo(E2E_ROOT, run.path)
  if (
    result.exit_status !== 0 ||
    (result.errors?.length ?? 0) > 0 ||
    result.comparison?.matches !== true
  ) {
    const reasons = [...(result.errors ?? []), ...(result.comparison?.differences ?? [])]
      .filter(Boolean)
      .slice(0, 4)
      .join('; ')
    throw new E2eError(
      `e2e regression failed${reasons ? `: ${reasons}` : ''}; inspect ${runRelative}/result.toml`
    )
  }
  console.log(`e2e regression matched baseline; run saved at ${runRelative}.`)
}

export function resolveRecordedArtifact(relativeFolder, e2eRoot = E2E_ROOT) {
  if (typeof relativeFolder !== 'string' || relativeFolder.length === 0) {
    throw new E2eError('install needs a relative baseline/run artifact folder')
  }
  const normalised = assertSafeRelativePath(relativeFolder, 'install folder')
  const parts = normalised.split('/')
  let resultPath
  let containerPath
  let artifactName
  if (parts[0] === 'baseline' && parts.length === 2) {
    resultPath = join(e2eRoot, 'baseline', 'result.toml')
    containerPath = join(e2eRoot, 'baseline')
    artifactName = parts[1]
  } else if (parts[0] === 'runs' && parts.length === 3) {
    resultPath = join(e2eRoot, 'runs', parts[1], 'result.toml')
    containerPath = join(e2eRoot, 'runs', parts[1])
    artifactName = parts[2]
  } else {
    throw new E2eError(
      'install folder must be exactly baseline/<mod-folder> or runs/<timestamp>/<mod-folder>'
    )
  }
  const artifactPath = resolve(e2eRoot, fromPosix(normalised))
  if (!isInside(e2eRoot, artifactPath)) {
    throw new E2eError(`install folder escapes r/e2e: ${relativeFolder}`)
  }
  return { normalised, resultPath, containerPath, artifactPath, artifactName }
}

function findRecordedMod(result, artifactName) {
  const matches = (result.mods ?? []).filter(
    (mod) => mod.artifact === artifactName || mod.name === artifactName
  )
  if (matches.length !== 1) {
    throw new E2eError(`artifact ${artifactName} is not recorded in ${result.kind} result.toml`)
  }
  return matches[0]
}

export async function installArtifact(relativeFolder, configPath = CONFIG_PATH) {
  const config = await loadConfig(configPath)
  await validateGameRoot(config.game)
  const location = resolveRecordedArtifact(relativeFolder)
  await requireDirectory(location.containerPath, 'recorded result directory')
  const result = await loadResult(location.resultPath)
  const expectedContainer = location.normalised.startsWith('baseline/') ? 'baseline' : 'run'
  if (result.kind !== expectedContainer) {
    throw new E2eError(`result kind does not match install folder ${relativeFolder}`)
  }
  const mod = findRecordedMod(result, location.artifactName)
  await requireDirectory(location.artifactPath, 'recorded artifact')
  const dataPath = join(config.game, 'data')
  const recordedFiles = manifestMap(mod.files)
  const groupFiles = new Set()
  for (const group of mod.patch_groups ?? []) {
    for (const path of group.files ?? [])
      groupFiles.add(assertSafeRelativePath(path, 'recorded patch path'))
  }
  if (groupFiles.size === 0) {
    throw new E2eError(`artifact ${relativeFolder} has no recorded patch-group files`)
  }

  let installed = 0
  for (const path of [...groupFiles].sort()) {
    const expected = recordedFiles.get(path)
    if (!expected)
      throw new E2eError(`recorded patch path is absent from the artifact manifest: ${path}`)
    const source = resolve(location.artifactPath, fromPosix(path))
    if (!isInside(location.artifactPath, source)) {
      throw new E2eError(`recorded patch path escapes its artifact: ${path}`)
    }
    await requireRegularFile(source, 'recorded patch file')
    const actualHash = await sha256File(source)
    const actualInfo = await lstat(source)
    if (actualHash !== expected.sha256 || actualInfo.size !== expected.size) {
      throw new E2eError(`recorded artifact was modified after the run: ${path}`)
    }
    const destination = resolve(dataPath, fromPosix(path))
    if (!isInside(dataPath, destination)) {
      throw new E2eError(`recorded patch path escapes game data: ${path}`)
    }
    await mkdir(dirname(destination), { recursive: true })
    await copyFile(source, destination)
    installed += 1
  }
  console.log(`Installed ${installed} recorded patch-group file(s) from ${relativeFolder}.`)
  return installed
}

async function removeArtifactDirectory(path) {
  const info = await existingPath(path)
  if (!info) return
  if (info.isSymbolicLink() || !info.isDirectory()) {
    throw new E2eError(`refusing to remove non-directory e2e artifact path: ${path}`)
  }
  await rm(path, { recursive: true, force: true })
}

export async function cleanArtifacts(e2eRoot = E2E_ROOT, all = false) {
  await removeArtifactDirectory(join(e2eRoot, 'runs'))
  if (all) await removeArtifactDirectory(join(e2eRoot, 'baseline'))
}

async function cleanCommand(all) {
  await cleanArtifacts(E2E_ROOT, all)
  console.log(all ? 'Removed e2e runs and baseline.' : 'Removed e2e runs.')
}

function parseArguments(argv) {
  const [command = 'help', ...rest] = argv
  const options = { command, positionals: [], force: false, all: false }
  for (let index = 0; index < rest.length; index += 1) {
    const argument = rest[index]
    if (argument === '--force') {
      options.force = true
    } else if (argument === '--all') {
      options.all = true
    } else if (argument === '--game' || argument === '--mods') {
      const value = rest[index + 1]
      if (!value || value.startsWith('--')) throw new E2eError(`${argument} needs a value`)
      options[argument.slice(2)] = value
      index += 1
    } else if (argument.startsWith('--game=') || argument.startsWith('--mods=')) {
      const separator = argument.indexOf('=')
      options[argument.slice(2, separator)] = argument.slice(separator + 1)
    } else if (argument.startsWith('--')) {
      throw new E2eError(`unknown option: ${argument}`)
    } else {
      options.positionals.push(argument)
    }
  }
  return options
}

function printHelp() {
  console.log(`Usage:
  bun run test:e2e:init [--game PATH] [--mods PATH]
  bun run test:e2e:baseline [--force]
  bun run test:e2e
  bun run test:e2e:install RELATIVE_FOLDER
  bun run test:e2e:clean [--all]

All persistent state is kept below ${E2E_ROOT}.`)
}

export async function runCommand(argv) {
  const options = parseArguments(argv)
  switch (options.command) {
    case 'init':
      if (options.positionals.length > 0)
        throw new E2eError('init does not accept positional arguments')
      return initCommand(options)
    case 'baseline':
      if (options.positionals.length > 0 || options.all) {
        throw new E2eError('baseline accepts only --force')
      }
      return baselineCommand(options.force)
    case 'test':
      if (options.positionals.length > 0 || options.force || options.all) {
        throw new E2eError('test does not accept options or positional arguments')
      }
      return testCommand()
    case 'install':
      if (options.force || options.all || options.positionals.length !== 1) {
        throw new E2eError('install needs exactly one relative artifact folder')
      }
      return installArtifact(options.positionals[0])
    case 'clean':
      if (options.positionals.length > 0 || options.force) {
        throw new E2eError('clean accepts only --all')
      }
      return cleanCommand(options.all)
    case 'help':
    case '--help':
    case '-h':
      printHelp()
      return undefined
    default:
      throw new E2eError(`unknown e2e command: ${options.command}`)
  }
}

const isMainModule = process.argv[1] && resolve(process.argv[1]) === TOOL_FILE
if (isMainModule) {
  try {
    await runCommand(process.argv.slice(2))
  } catch (error) {
    console.error(`e2e error: ${error instanceof Error ? error.message : String(error)}`)
    process.exitCode = 1
  }
}
