param([int]$ParentId, [string]$Stage, [string]$Target, [string]$Backup, [string]$Work)
$ErrorActionPreference = 'Stop'
Wait-Process -Id $ParentId -ErrorAction SilentlyContinue
Move-Item -LiteralPath $Target -Destination $Backup
try {
    Move-Item -LiteralPath $Stage -Destination $Target
    Start-Process -FilePath $Target
} catch {
    if (Test-Path -LiteralPath $Target) { Move-Item -LiteralPath $Target -Destination $Stage }
    Move-Item -LiteralPath $Backup -Destination $Target
    Start-Process -FilePath $Target
    throw
}
Remove-Item -LiteralPath $Work -Recurse -Force
