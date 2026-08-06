import { fileURLToPath, URL } from 'node:url'
import { lingui, linguiTransformerBabelPreset } from '@lingui/vite-plugin'
import babel from '@rolldown/plugin-babel'
import tailwindcss from '@tailwindcss/vite'
import react, { reactCompilerPreset } from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// @vitejs/plugin-react v6 (Vite 8) transpiles with oxc and no longer runs Babel,
// so the Lingui macros and the React Compiler run through @rolldown/plugin-babel.
// Babel applies presets in reverse order, so listing the compiler preset first
// makes the Lingui macro preset run first — macros must expand before the
// compiler analyses the code.
export default defineConfig({
  plugins: [
    react(),
    lingui(),
    babel({ presets: [reactCompilerPreset(), linguiTransformerBabelPreset()] }),
    tailwindcss(),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
})
