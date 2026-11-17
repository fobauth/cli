# Installs the latest fob for Windows into %LOCALAPPDATA%\Programs\fob and puts it on your PATH.
& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'

    $base = 'https://github.com/fobauth/cli/releases/latest/download'
    $archive = 'fob-x86_64-pc-windows-msvc.zip'
    $dir = Join-Path $env:LOCALAPPDATA 'Programs\fob'

    if (-not [Environment]::Is64BitOperatingSystem) {
        throw 'There is no fob for 32-bit Windows.'
    }

    # Windows PowerShell 5.1 can default to TLS versions GitHub refuses.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("fob-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $zip = Join-Path $tmp $archive
        $sums = Join-Path $tmp 'SHA256SUMS'
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$archive" -OutFile $zip
        Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile $sums

        $expected = @(Get-Content $sums | ForEach-Object {
            $hash, $name = $_.Trim() -split '\s+', 2
            if ($name -and $name.TrimStart('*') -eq $archive) { $hash }
        })
        if ($expected.Count -ne 1) {
            throw "SHA256SUMS doesn't list $archive."
        }
        $actual = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash
        if (-not $actual -or $actual -ne $expected[0]) {
            throw "$archive doesn't match its checksum. Nothing was installed."
        }

        Expand-Archive -Path $zip -DestinationPath (Join-Path $tmp 'fob') -Force
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        Copy-Item -Path (Join-Path $tmp 'fob\fob.exe') -Destination (Join-Path $dir 'fob.exe') -Force
    }
    finally {
        Remove-Item -Recurse -Force $tmp
    }

    $version = & (Join-Path $dir 'fob.exe') --version
    Write-Host "Installed $version to $dir\fob.exe"

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $onPath = @($userPath -split ';' | Where-Object { $_.TrimEnd('\') -eq $dir })
    if ($onPath.Count -eq 0) {
        $newPath = (@($userPath -split ';' | Where-Object { $_ }) + $dir) -join ';'
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        $env:Path = "$env:Path;$dir"
        Write-Host "Added $dir to your PATH. Open a new terminal to run fob by name."
    }
}
