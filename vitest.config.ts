import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [sveltekit()],
  // Component tests mount Svelte into happy-dom, so use Svelte's client entry
  // instead of the default SSR entry used by ordinary Node tests.
  resolve: { conditions: ['browser'] }
});
