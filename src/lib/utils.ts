export type ClassValue = string | false | null | undefined

/** Minimal class-name joiner (shadcn's `cn`, without tailwind-merge). */
export function cn(...classes: ClassValue[]): string {
  return classes.filter(Boolean).join(' ')
}

/** Last path segment, for showing a folder's name from its absolute path. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean)
  return parts[parts.length - 1] ?? path
}
