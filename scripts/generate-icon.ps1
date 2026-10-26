# Rebuild the transparent multi-resolution icon from Scythe's original blade/handle geometry.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$iconRoot = Split-Path $PSScriptRoot -Parent
$iconSizes = @(16,20,24,32,40,48,64,128,256)
$iconImages = @()
foreach ($iconSize in $iconSizes) {
    $iconBitmap = New-Object System.Drawing.Bitmap($iconSize, $iconSize, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $iconGraphics = [System.Drawing.Graphics]::FromImage($iconBitmap)
    $iconGraphics.Clear([System.Drawing.Color]::Transparent)
    $iconGraphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $iconGraphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $iconGraphics.ScaleTransform(($iconSize / 32.0), ($iconSize / 32.0))
    $iconBlade = [System.Drawing.PointF[]]@(
        [System.Drawing.PointF]::new(4,12), [System.Drawing.PointF]::new(9,5),
        [System.Drawing.PointF]::new(18,3), [System.Drawing.PointF]::new(27,7),
        [System.Drawing.PointF]::new(30,14), [System.Drawing.PointF]::new(23,10),
        [System.Drawing.PointF]::new(15,9), [System.Drawing.PointF]::new(8,12))
    $iconHandle = [System.Drawing.PointF[]]@(
        [System.Drawing.PointF]::new(21,9), [System.Drawing.PointF]::new(24,11),
        [System.Drawing.PointF]::new(11,30), [System.Drawing.PointF]::new(8,28))
    $iconBlue = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255,34,116,165))
    $iconNeutral = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255,233,241,247))
    $iconOutline = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(210,39,55,68),0.55)
    $iconGraphics.FillPolygon($iconBlue,$iconBlade)
    $iconGraphics.FillPolygon($iconNeutral,$iconHandle)
    $iconGraphics.DrawPolygon($iconOutline,$iconHandle)
    $iconStream = [System.IO.MemoryStream]::new()
    $iconBitmap.Save($iconStream,[System.Drawing.Imaging.ImageFormat]::Png)
    $iconImages += ,$iconStream.ToArray()
    $iconStream.Dispose()
    $iconOutline.Dispose()
    $iconNeutral.Dispose()
    $iconBlue.Dispose()
    $iconGraphics.Dispose()
    $iconBitmap.Dispose()
}
$iconDestination = Join-Path $iconRoot 'assets/scythe.ico'
$iconFile = [System.IO.File]::Create($iconDestination)
$iconWriter = [System.IO.BinaryWriter]::new($iconFile)
$iconWriter.Write([uint16]0)
$iconWriter.Write([uint16]1)
$iconWriter.Write([uint16]$iconSizes.Count)
$iconOffset = 6 + 16 * $iconSizes.Count
for ($iconIndex = 0; $iconIndex -lt $iconSizes.Count; $iconIndex++) {
    $iconDimension = if ($iconSizes[$iconIndex] -eq 256) { 0 } else { $iconSizes[$iconIndex] }
    $iconWriter.Write([byte]$iconDimension)
    $iconWriter.Write([byte]$iconDimension)
    $iconWriter.Write([byte]0)
    $iconWriter.Write([byte]0)
    $iconWriter.Write([uint16]1)
    $iconWriter.Write([uint16]32)
    $iconWriter.Write([uint32]$iconImages[$iconIndex].Length)
    $iconWriter.Write([uint32]$iconOffset)
    $iconOffset += $iconImages[$iconIndex].Length
}
foreach ($iconImage in $iconImages) { $iconWriter.Write([byte[]]$iconImage) }
$iconWriter.Dispose()
