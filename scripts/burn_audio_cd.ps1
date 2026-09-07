<#
.SYNOPSIS
  Burns a Red Book (CDDA) audio CD from raw PCM track files using Windows'
  built-in IMAPI2 Track-At-Once COM API. No third-party burning software
  required.

.PARAMETER TrackFiles
  Comma-separated list of raw PCM files: 44100Hz, 16-bit signed little-endian,
  2 channels, NO container/header (i.e. a WAV with the RIFF header stripped).
  ytmusicdw produces these itself before calling this script.

.NOTES
  Insert a blank CD-R before running. This performs a single Track-At-Once
  session and closes the disc so it's playable in standalone car stereos.
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$TrackFiles
)

$ErrorActionPreference = "Stop"

$paths = $TrackFiles -split "," | Where-Object { $_.Trim() -ne "" }
if ($paths.Count -eq 0) {
    throw "No track files supplied."
}
foreach ($p in $paths) {
    if (-not (Test-Path $p)) {
        throw "Track file not found: $p"
    }
}

Write-Host "Looking for a CD/DVD recorder..."
$discMaster = New-Object -ComObject IMAPI2.MsftDiscMaster2
if ($discMaster.Count -eq 0) {
    throw "No CD/DVD recorder found. Plug in the burner and try again."
}

$recorderId = $discMaster.Item(0)
$recorder = New-Object -ComObject IMAPI2.MsftDiscRecorder2
$recorder.InitializeDiscRecorder($recorderId)
Write-Host "Using recorder: $($recorder.ProductId)"

$format = New-Object -ComObject IMAPI2.MsftDiscFormat2TrackAtOnce
$format.Recorder = $recorder
$format.ClientName = "ytmusicdw"

if (-not $format.IsRecorderSupported($recorder)) {
    throw "This recorder does not support Track-At-Once audio burning."
}
if (-not $format.IsCurrentMediaSupported($recorder)) {
    throw "The disc in the drive isn't usable for an audio CD. Insert a blank CD-R and retry."
}

Write-Host "Preparing media (erasing/formatting if needed)..."
$format.PrepareMedia()

$i = 0
foreach ($path in $paths) {
    $i++
    Write-Host "Adding track $i of $($paths.Count): $path"

    $stream = New-Object -ComObject ADODB.Stream
    $stream.Type = 1  # adTypeBinary
    $stream.Open()
    $stream.LoadFromFile($path)

    $format.AddAudioTrack($stream)
}

Write-Host "Writing disc (do not remove it)..."
$format.Write()

Write-Host "Ejecting..."
try { $recorder.EjectMedia() } catch { }

Write-Host "Done. Audio CD burned successfully."
