import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';

const configPath = resolve(process.cwd(), 'src-tauri', 'tauri.conf.json');
const config = JSON.parse(readFileSync(configPath, 'utf8')) as {
  mainBinaryName?: string;
  build: { devUrl?: string; frontendDist?: string; beforeBuildCommand?: string };
};

describe('production Tauri frontend configuration', () => {
  it('uses a local bundled frontend distribution instead of the development URL', () => {
    expect(config.build.devUrl).toMatch(/^http:\/\/localhost:/);
    expect(config.build.frontendDist).toBeTypeOf('string');
    expect(config.build.frontendDist).not.toMatch(/^https?:\/\//i);
    expect(config.build.beforeBuildCommand).toBe('npm run build');

    const frontendDist = resolve(dirname(configPath), config.build.frontendDist!);
    expect(existsSync(frontendDist)).toBe(true);
    expect(existsSync(resolve(frontendDist, 'index.html'))).toBe(true);
  });

  it('declares the Tauri application binary explicitly', () => {
    expect(config.mainBinaryName).toBe('wvu_bioinstrumentation_studio');
  });
});
