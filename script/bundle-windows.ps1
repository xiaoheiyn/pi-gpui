$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    $target = 'x86_64-pc-windows-msvc'
    cargo build --release --locked --target $target
    if ($LASTEXITCODE -ne 0) { throw 'Windows release build failed.' }

    $version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $release = Join-Path 'target' "$target/release"
    $app = Join-Path $release 'Pi GPUI'
    New-Item -ItemType Directory -Path $app -Force | Out-Null
    Copy-Item (Join-Path $release 'pi-gpui.exe') (Join-Path $app 'Pi GPUI.exe') -Force
    Copy-Item 'assets/fonts/OFL.txt' (Join-Path $app 'JetBrainsMono-OFL.txt') -Force
    Copy-Item 'LICENSE' $app -Force
    $files = Get-ChildItem -LiteralPath $app -File | Where-Object { $_.Name -in @('Pi GPUI.exe', 'JetBrainsMono-OFL.txt', 'LICENSE') }
    $zip = Join-Path $release "Pi-GPUI-$version-windows-x64.zip"
    Compress-Archive -LiteralPath $files.FullName -DestinationPath $zip -Force
    Write-Output $zip
} finally {
    Pop-Location
}
