# Phát hành một bản: build có ký -> latest.json -> GitHub Release.
# Dùng:  powershell -File scripts\release.ps1 [-Notes "ghi chú" | -NotesFile ghichu.txt]
# Phiên bản lấy từ src-tauri\tauri.conf.json (nâng version TRƯỚC, ở cả package.json + Cargo.toml).
# Commit + push TRƯỚC khi chạy: gh tạo tag ở HEAD trên GitHub, chưa push thì tag trỏ commit cũ.
# Cần: gh đã đăng nhập, private key updater ở ~/.tauri/vinarrator_updater.key (KHÔNG commit, mất là
# người dùng cũ không cập nhật được nữa).
# -NotesFile cho ghi chú nhiều dòng / có dấu ngoặc kép: PowerShell 5.1 làm vỡ dấu " khi truyền
# chuỗi cho chương trình ngoài như gh.
param([string]$Notes = '', [string]$NotesFile = '')
$ErrorActionPreference = 'Stop'
if ($NotesFile) {
  $NotesFile = (Resolve-Path $NotesFile).Path
  $Notes = [IO.File]::ReadAllText($NotesFile, [Text.Encoding]::UTF8).Trim()
}
Set-Location (Split-Path $PSScriptRoot)

$ver = (Get-Content src-tauri\tauri.conf.json -Raw | ConvertFrom-Json).version
$tag = "v$ver"
$key = Join-Path $HOME '.tauri\vinarrator_updater.key'
if (-not (Test-Path $key)) { throw "Thiếu private key: $key" }
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content (Join-Path $HOME '.tauri\vinarrator_updater.pass') -Raw).Trim()

yarn build
if ($LASTEXITCODE) { throw 'build lỗi' }

$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'src-tauri\target' }
$exe = Join-Path $target "release\bundle\nsis\ViNarrator_${ver}_x64-setup.exe"
$sig = "$exe.sig"
if (-not (Test-Path $sig)) { throw "Không thấy chữ ký $sig (createUpdaterArtifacts?)" }

$json = [ordered]@{
  version  = $ver
  notes    = $Notes
  pub_date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
  platforms = @{
    'windows-x86_64' = @{
      signature = (Get-Content $sig -Raw).Trim()
      url       = "https://github.com/din730199/ViNarrator/releases/download/$tag/ViNarrator_${ver}_x64-setup.exe"
    }
  }
} | ConvertTo-Json -Depth 5
$latest = Join-Path $env:TEMP 'vinarrator-latest.json'
[IO.File]::WriteAllText($latest, $json, (New-Object Text.UTF8Encoding $false))

# latest.json phải mang đúng tên này: endpoint trỏ tới releases/latest/download/latest.json.
$tmp = Join-Path $env:TEMP 'vinarrator-release'
New-Item -ItemType Directory -Force $tmp | Out-Null
Copy-Item $latest (Join-Path $tmp 'latest.json') -Force
$ghiChu = Join-Path $tmp 'notes.txt'
[IO.File]::WriteAllText($ghiChu, $(if ($Notes) { $Notes } else { "ViNarrator $tag" }), (New-Object Text.UTF8Encoding $false))
gh release create $tag $exe (Join-Path $tmp 'latest.json') --title "ViNarrator $tag" --notes-file $ghiChu
