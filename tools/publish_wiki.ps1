param(
  [Parameter(Mandatory = $false)]
  [string]$RepoSlug = "Steven9101/NovaSDR"
)

$ErrorActionPreference = "Stop"

function New-TempDirectory {
  $tempRoot = if ($env:TEMP) { $env:TEMP } else { [System.IO.Path]::GetTempPath() }
  $path = Join-Path $tempRoot ("novasdr-wiki-" + [Guid]::NewGuid().ToString("N"))
  New-Item -ItemType Directory -Path $path | Out-Null
  return $path
}

function Clear-DirectoryExceptGit {
  param([Parameter(Mandatory = $true)][string]$Path)

  Get-ChildItem -LiteralPath $Path -Force | Where-Object { $_.Name -ne ".git" } | ForEach-Object {
    Remove-Item -LiteralPath $_.FullName -Recurse -Force
  }
}

function Copy-DirectoryContents {
  param(
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$Destination
  )

  if (-not (Test-Path -LiteralPath $Source)) {
    return
  }

  New-Item -ItemType Directory -Path $Destination -Force | Out-Null

  $items = Get-ChildItem -LiteralPath $Source -Force
  foreach ($item in $items) {
    $target = Join-Path $Destination $item.Name
    Copy-Item -LiteralPath $item.FullName -Destination $target -Recurse -Force
  }
}

$wikiUrl = "https://github.com/$RepoSlug.wiki.git"
$tmpDir = New-TempDirectory

try {
  $wikiDir = Join-Path $tmpDir "wiki"

  git clone $wikiUrl $wikiDir | Out-Null
  if (($LASTEXITCODE -ne 0) -or (-not (Test-Path -LiteralPath $wikiDir))) {
    [Console]::Error.WriteLine(@"
Failed to clone the wiki repository: $wikiUrl

This usually means the GitHub wiki is disabled for the repo or you don't have access.
Enable it in: Settings -> Features -> Wikis, then rerun:
  powershell -ExecutionPolicy Bypass -File tools/publish_wiki.ps1 -RepoSlug $RepoSlug
"@)
    exit 2
  }

  Clear-DirectoryExceptGit -Path $wikiDir

  Copy-DirectoryContents -Source (Join-Path $PWD.Path "docs/wiki") -Destination $wikiDir
  Copy-DirectoryContents -Source (Join-Path $PWD.Path "docs/assets") -Destination (Join-Path $wikiDir "assets")

  $docs = Get-ChildItem -LiteralPath (Join-Path $PWD.Path "docs") -Filter *.md -File
  foreach ($doc in $docs) {
    $name = [System.IO.Path]::GetFileNameWithoutExtension($doc.Name)
    $out = if ($name -eq "index") { Join-Path $wikiDir "Home.md" } else { Join-Path $wikiDir ($name + ".md") }

    $text = Get-Content -LiteralPath $doc.FullName -Raw -Encoding utf8
    $text = $text -replace '\]\(index\.md\)', '](Home)'
    $text = $text -replace '\]\(([^)]+)\.md\)', ']($1)'

    [System.IO.File]::WriteAllText($out, $text, (New-Object System.Text.UTF8Encoding($false)))
  }

  Push-Location $wikiDir
  try {
    $status = git status --porcelain=v1
    if ($status) {
      git add -A | Out-Null
      git commit -m "Sync wiki from repo docs" | Out-Null
      git push origin HEAD | Out-Null
      Write-Host "Wiki updated."
    } else {
      Write-Host "Wiki already up to date."
    }
  } finally {
    Pop-Location
  }
} finally {
  Remove-Item -LiteralPath $tmpDir -Recurse -Force
}
