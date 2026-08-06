import type { HTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

type Variant = 'default' | 'muted' | 'success' | 'destructive' | 'outline'

const variants: Record<Variant, string> = {
  default: 'bg-primary/15 text-foreground',
  muted: 'bg-muted text-muted-foreground',
  success: 'text-[var(--success)] bg-[color-mix(in_oklch,var(--success)_18%,transparent)]',
  destructive: 'bg-destructive/15 text-destructive',
  outline: 'border border-border text-muted-foreground',
}

export function Badge({
  className,
  variant = 'default',
  ...props
}: HTMLAttributes<HTMLSpanElement> & { variant?: Variant }) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-xs font-medium',
        variants[variant],
        className
      )}
      {...props}
    />
  )
}
