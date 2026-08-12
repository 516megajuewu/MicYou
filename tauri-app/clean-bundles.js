import { execFileSync } from 'node:child_process';
import { rmSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const appDir = path.dirname(fileURLToPath(import.meta.url));
const metadata = JSON.parse(
  execFileSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], {
    cwd: appDir,
    encoding: 'utf8',
  }),
);
const targetDir = path.resolve(metadata.target_directory);
const target = process.env.TAURI_ENV_TARGET_TRIPLE;
const bundleDirs = [path.join(targetDir, 'release', 'bundle')];
if (target) bundleDirs.push(path.join(targetDir, target, 'release', 'bundle'));

for (const bundleDir of new Set(bundleDirs)) {
  const resolved = path.resolve(bundleDir);
  if (!resolved.startsWith(targetDir + path.sep)) {
    throw new Error('Refusing to clean a bundle directory outside Cargo target');
  }
  rmSync(resolved, { recursive: true, force: true });
  console.log('[bundle] Cleaned ' + path.relative(appDir, resolved));
}
