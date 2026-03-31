import { readFileSync, writeFileSync } from 'fs'

const type = process.argv[2] || 'patch'

const cargo = readFileSync('Cargo.toml', 'utf8')
const match = cargo.match(/^version = "(\d+)\.(\d+)\.(\d+)"/m)
if (!match) {
  console.error('无法从 Cargo.toml 读取版本号')
  process.exit(1)
}

let [, major, minor, patch] = match.map(Number)

switch (type) {
  case 'major': major++; minor = 0; patch = 0; break
  case 'minor': minor++; patch = 0; break
  case 'patch': patch++; break
  default:
    if (/^\d+\.\d+\.\d+$/.test(type)) {
      ;[major, minor, patch] = type.split('.').map(Number)
    } else {
      console.error(`用法: pnpm bump [patch|minor|major|x.y.z]\n  当前: ${match[1]}.${match[2]}.${match[3]}`)
      process.exit(1)
    }
}

const version = `${major}.${minor}.${patch}`
const date = new Date().toISOString().slice(0, 10)

const files = [
  { path: 'Cargo.toml', replace: (s) => s.replace(/^version = "[^"]*"/m, `version = "${version}"`) },
  { path: 'package.json', replace: (s) => s.replace(/"version"\s*:\s*"[^"]*"/, `"version": "${version}"`) },
  { path: 'tauri.conf.json', replace: (s) => s.replace(/"version"\s*:\s*"[^"]*"/, `"version": "${version}"`) },
  { path: 'version.json', replace: (s) => s.replace(/"version"\s*:\s*"[^"]*"/, `"version": "${version}"`).replace(/"build_date"\s*:\s*"[^"]*"/, `"build_date": "${date}"`) },
]

for (const { path, replace } of files) {
  try {
    const content = readFileSync(path, 'utf8')
    const updated = replace(content)
    if (content !== updated) {
      writeFileSync(path, updated)
      console.log(`  ✓ ${path}`)
    }
  } catch {}
}

console.log(`\n  版本: ${match[1]}.${match[2]}.${match[3]} → ${version}`)
