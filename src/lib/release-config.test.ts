import { describe, expect, it } from 'vitest';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';

const configPath = resolve(process.cwd(), 'src-tauri', 'tauri.conf.json');
const config = JSON.parse(readFileSync(configPath, 'utf8')) as {
  mainBinaryName?: string;
  build: { devUrl?: string; frontendDist?: string; beforeBuildCommand?: string };
  app?: { security?: { csp?: string | null } };
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

  it('opens a centred window that fits a 1366×768 lab PC, title bar and taskbar included', () => {
    const window = (config.app as { windows?: Array<Record<string, number | boolean>> } | undefined)?.windows?.[0];
    expect(window).toMatchObject({ width: 1280, height: 680, minWidth: 960, minHeight: 600, center: true });
  });

  it('declares the Tauri application binary explicitly', () => {
    expect(config.mainBinaryName).toBe('wvu_bioinstrumentation_studio');
  });

  it('uses a restrictive production content-security policy', () => {
    const csp = config.app?.security?.csp;
    expect(csp).toContain("default-src 'self'");
    expect(csp).toContain("connect-src 'self' ipc: http://ipc.localhost");
    expect(csp).not.toContain("script-src *");
    // The asset protocol is not enabled or used, so images come only from
    // the bundle or inline data.
    expect(csp).toContain("img-src 'self' data:;");
    expect(csp).not.toContain('asset:');
  });

  it('grants only the dialog commands the frontend uses', () => {
    // open (Project folder, lab import), save (lab export), and confirm,
    // which plugin-dialog 2.x sends as its `message` command.
    const capability = JSON.parse(
      readFileSync(resolve(dirname(configPath), 'capabilities', 'default.json'), 'utf8')
    ) as { permissions: string[] };
    const dialog = capability.permissions.filter((permission) => permission.startsWith('dialog:'));
    expect(dialog.sort()).toEqual(['dialog:allow-message', 'dialog:allow-open', 'dialog:allow-save']);
  });

  it('never uses the browser dialogs that plugin-dialog replaces', () => {
    // plugin-dialog's init script turns window.confirm into an async function
    // that sends an unregistered command: its Promise is always truthy, so
    // `if (!window.confirm(...)) return;` would never cancel. Use the
    // plugin's confirm/message instead.
    const offenders: string[] = [];
    const visit = (folder: string) => {
      for (const entry of readdirSync(folder, { withFileTypes: true })) {
        const path = resolve(folder, entry.name);
        if (entry.isDirectory()) visit(path);
        else if (/\.(svelte|ts)$/.test(entry.name) && !/\.test\.ts$/.test(entry.name)) {
          // A bare confirm( is the plugin's, imported; alert( and prompt( have no import.
          const text = readFileSync(path, 'utf8');
          if (/\b(?:window|globalThis|self)\.(?:alert|confirm|prompt)\s*\(|(?<![\w.$])(?:alert|prompt)\s*\(/.test(text)) offenders.push(path);
        }
      }
    };
    visit(resolve(process.cwd(), 'src'));
    expect(offenders).toEqual([]);
  });
});
