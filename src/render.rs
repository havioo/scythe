//! GDI+ primitives in logical pixels, rasterized directly at the target's scale.
//! HDCs must use MM_TEXT with no additional GDI world transform. No graphics
//! object survives a draw call, so callers can interleave GDI reads and blits.
use std::{
    cell::RefCell,
    marker::PhantomData,
    ptr::null_mut,
    rc::Rc,
    sync::{Mutex, OnceLock},
};
use windows_sys::Win32::{
    Foundation::{POINT, RECT},
    Graphics::{Gdi::HDC, GdiPlus::*},
};

static REGULAR: &[u8] = include_bytes!("../assets/fonts/ClearSans-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../assets/fonts/ClearSans-Bold.ttf");
static STARTUP: OnceLock<Result<usize, Status>> = OnceLock::new();
static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();

struct Fonts {
    // Native objects and backing bytes intentionally remain alive until exit.
    _collection: usize,
    family: usize,
    private: bool,
    diagnostic: String,
}

fn started() -> bool {
    STARTUP
        .get_or_init(|| unsafe {
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                DebugEventCallback: 0,
                SuppressBackgroundThread: 0,
                SuppressExternalCodecs: 0,
            };
            let mut token = 0;
            let status = GdiplusStartup(&mut token, &input, null_mut());
            if status == Ok {
                Result::Ok(token)
            } else {
                Err(status)
            }
        })
        .is_ok()
}

fn fonts() -> &'static Mutex<Fonts> {
    FONTS.get_or_init(|| Mutex::new(unsafe {
        let mut result = Fonts { _collection: 0, family: 0, private: false,
            diagnostic: "GDI+ startup failed".into() };
        if !started() { return Mutex::new(result); }
        let mut collection = null_mut();
        let mut status = GdipNewPrivateFontCollection(&mut collection);
        result._collection = collection as usize;
        if status == Ok {
            for bytes in [REGULAR, BOLD] {
                status = GdipPrivateAddMemoryFont(collection, bytes.as_ptr().cast(), bytes.len() as i32);
                if status != Ok { break; }
            }
        }
        let mut family = null_mut();
        if status == Ok {
            let name: Vec<u16> = "Clear Sans\0".encode_utf16().collect();
            status = GdipCreateFontFamilyFromName(name.as_ptr(), collection, &mut family);
        }
        if status == Ok {
            for style in [FontStyleRegular, FontStyleBold] {
                let mut available = 0;
                status = GdipIsStyleAvailable(family, style, &mut available);
                if status != Ok || available == 0 {
                    if status == Ok { status = FontStyleNotFound; }
                    break;
                }
                // Verify that GDI+ can instantiate each embedded face as well.
                let mut font = null_mut();
                status = GdipCreateFont(family, 16.0, style, UnitPixel, &mut font);
                if status != Ok { break; }
                GdipDeleteFont(font);
            }
        }
        if status == Ok {
            result.private = true;
            result.diagnostic = "Private embedded Clear Sans Regular and Bold loaded".into();
        } else {
            if !family.is_null() { GdipDeleteFontFamily(family); family = null_mut(); }
            let fallback = GdipGetGenericFontFamilySansSerif(&mut family);
            result.diagnostic = format!("Private Clear Sans load failed (GDI+ status {status}); generic sans fallback status {fallback}");
            eprintln!("{}", result.diagnostic);
        }
        result.family = family as usize;
        result
    }))
}

/// True only after both private embedded faces have loaded and instantiated.
pub fn font_ready() -> bool {
    fonts().lock().unwrap_or_else(|e| e.into_inner()).private
}

/// Reports the actual selected family source, including load failure.
pub fn font_name() -> &'static str {
    let fonts = fonts().lock().unwrap_or_else(|e| e.into_inner());
    if fonts.private {
        "Clear Sans"
    } else if fonts.family != 0 {
        "Generic sans-serif (Clear Sans load failed)"
    } else {
        "Unavailable (GDI+ or font load failed)"
    }
}

/// Detailed startup/font-load diagnostic suitable for a CLI information command.
pub fn font_diagnostics() -> String {
    fonts()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .diagnostic
        .clone()
}

struct Registration {
    id: Rc<()>,
    dc: usize,
    sx: f32,
    sy: f32,
    tx: f32,
}
thread_local! { static TARGETS: RefCell<Vec<Registration>> = const { RefCell::new(Vec::new()) }; }

/// Thread-bound scope registering scale only; it owns no HDC or GpGraphics.
#[must_use = "keep the guard alive while drawing the target"]
pub struct TargetGuard {
    id: Rc<()>,
    _thread: PhantomData<Rc<()>>,
}
impl Drop for TargetGuard {
    fn drop(&mut self) {
        let _ = TARGETS.try_with(|targets| {
            targets
                .borrow_mut()
                .retain(|r| !Rc::ptr_eq(&r.id, &self.id))
        });
    }
}

/// Registers physical/logical scaling. Nested scopes restore the previous scale,
/// even when guards are dropped out of order. Invalid dimensions use scale 1.
/// # Safety
/// `dc` must remain a valid, exclusively used HDC for the guard's lifetime.
pub unsafe fn target(
    dc: HDC,
    pixel_width: i32,
    pixel_height: i32,
    logical_width: i32,
    logical_height: i32,
) -> TargetGuard {
    let id = Rc::new(());
    let (sx, sy) = if pixel_width > 0 && pixel_height > 0 && logical_width > 0 && logical_height > 0
    {
        (
            pixel_width as f32 / logical_width as f32,
            pixel_height as f32 / logical_height as f32,
        )
    } else {
        (1.0, 1.0)
    };
    TARGETS.with(|targets| {
        targets.borrow_mut().push(Registration {
            id: id.clone(),
            dc: dc as usize,
            sx,
            sy,
            tx: 0.0,
        })
    });
    TargetGuard {
        id,
        _thread: PhantomData,
    }
}

/// Temporarily translates logical drawing on an already registered target.
/// # Safety
/// `dc` must be valid and exclusively used for the scope.
pub unsafe fn translate(dc: HDC, x: f32) -> TargetGuard {
    let id = Rc::new(());
    TARGETS.with(|targets| {
        let mut targets = targets.borrow_mut();
        let (sx, sy, tx) = targets
            .iter()
            .rev()
            .find(|r| r.dc == dc as usize)
            .map_or((1.0, 1.0, 0.0), |r| (r.sx, r.sy, r.tx));
        targets.push(Registration {
            id: id.clone(),
            dc: dc as usize,
            sx,
            sy,
            tx: tx + x,
        });
    });
    TargetGuard {
        id,
        _thread: PhantomData,
    }
}

struct Graphics(*mut GpGraphics);
impl Graphics {
    unsafe fn new(dc: HDC) -> Option<Self> {
        unsafe {
            if dc.is_null() || !started() {
                return None;
            }
            let mut p = null_mut();
            if GdipCreateFromHDC(dc, &mut p) != Ok {
                return None;
            }
            let g = Self(p);
            let (sx, sy, tx) = TARGETS.with(|targets| {
                targets
                    .borrow()
                    .iter()
                    .rev()
                    .find(|r| r.dc == dc as usize)
                    .map_or((1.0, 1.0, 0.0), |r| (r.sx, r.sy, r.tx))
            });
            if GdipSetPageUnit(p, UnitPixel) != Ok
                || GdipScaleWorldTransform(p, sx, sy, MatrixOrderPrepend) != Ok
            {
                return None;
            }
            GdipTranslateWorldTransform(p, tx, 0.0, MatrixOrderPrepend);
            GdipSetSmoothingMode(p, SmoothingModeAntiAlias8x8);
            GdipSetCompositingQuality(p, CompositingQualityHighQuality);
            GdipSetTextRenderingHint(p, TextRenderingHintAntiAliasGridFit);
            Some(g)
        }
    }
}
impl Drop for Graphics {
    fn drop(&mut self) {
        unsafe {
            GdipFlush(self.0, FlushIntentionSync);
            GdipDeleteGraphics(self.0);
        }
    }
}

macro_rules! owned {
    ($name:ident, $native:ty, $delete:ident) => {
        struct $name(*mut $native);
        impl Drop for $name {
            fn drop(&mut self) {
                unsafe {
                    $delete(self.0.cast());
                }
            }
        }
    };
}
owned!(Brush, GpSolidFill, GdipDeleteBrush);
owned!(Path, GpPath, GdipDeletePath);
owned!(Font, GpFont, GdipDeleteFont);
owned!(Format, GpStringFormat, GdipDeleteStringFormat);
owned!(Bitmap, GpBitmap, GdipDisposeImage);
owned!(Attributes, GpImageAttributes, GdipDisposeImageAttributes);

fn argb(c: u32) -> u32 {
    0xff00_0000 | ((c & 255) << 16) | (c & 0xff00) | ((c >> 16) & 255)
}
fn bounds(r: RECT) -> Option<RectF> {
    let w = r.right as f32 - r.left as f32;
    let h = r.bottom as f32 - r.top as f32;
    (w > 0.0 && h > 0.0).then_some(RectF {
        X: r.left as f32,
        Y: r.top as f32,
        Width: w,
        Height: h,
    })
}
unsafe fn brush(color: u32) -> Option<Brush> {
    unsafe {
        let mut p = null_mut();
        if GdipCreateSolidFill(argb(color), &mut p) == Ok {
            Some(Brush(p))
        } else {
            None
        }
    }
}

/// Clears the full drawing target without antialiased rectangle edge seams.
/// # Safety
/// `dc` must be valid and exclusively used during the call.
pub unsafe fn clear(dc: HDC, color: u32) {
    unsafe {
        if let Some(g) = Graphics::new(dc) {
            GdipGraphicsClear(g.0, argb(color));
        }
    }
}

/// Fills a logical rectangle with an opaque COLORREF color.
/// # Safety
/// `dc` must be a valid HDC exclusively used during this call.
pub unsafe fn fill(dc: HDC, r: RECT, color: u32) {
    unsafe {
        let Some(r) = bounds(r) else {
            return;
        };
        let Some(g) = Graphics::new(dc) else {
            return;
        };
        let Some(b) = brush(color) else {
            return;
        };
        GdipFillRectangle(g.0, b.0.cast(), r.X, r.Y, r.Width, r.Height);
    }
}

/// Fills an antialiased rounded rectangle. `diameter` follows GDI RoundRect
/// semantics (corner radius = diameter / 2), clamped to the rectangle size.
/// # Safety
/// `dc` must be a valid HDC exclusively used during this call.
pub unsafe fn rounded(dc: HDC, r: RECT, color: u32, diameter: i32) {
    unsafe {
        if diameter <= 0 {
            fill(dc, r, color);
            return;
        }
        let Some(r) = bounds(r) else {
            return;
        };
        let Some(g) = Graphics::new(dc) else {
            return;
        };
        let Some(b) = brush(color) else {
            return;
        };
        let mut p = null_mut();
        if GdipCreatePath(FillModeWinding, &mut p) != Ok {
            return;
        }
        let p = Path(p);
        let d = (diameter as f32).min(r.Width).min(r.Height);
        for (x, y, angle) in [
            (r.X, r.Y, 180.0),
            (r.X + r.Width - d, r.Y, 270.0),
            (r.X + r.Width - d, r.Y + r.Height - d, 0.0),
            (r.X, r.Y + r.Height - d, 90.0),
        ] {
            if GdipAddPathArc(p.0, x, y, d, d, angle, 90.0) != Ok {
                return;
            }
        }
        GdipClosePathFigure(p.0);
        GdipFillPath(g.0, b.0.cast(), p.0);
    }
}

/// Fills an antialiased polygon in logical coordinates.
/// # Safety
/// `dc` must be a valid HDC exclusively used during this call.
pub unsafe fn polygon(dc: HDC, points: &[POINT], color: u32) {
    unsafe {
        if points.len() < 3 {
            return;
        }
        let Result::Ok(count) = i32::try_from(points.len()) else {
            return;
        };
        let Some(g) = Graphics::new(dc) else {
            return;
        };
        let Some(b) = brush(color) else {
            return;
        };
        let points: Vec<PointF> = points
            .iter()
            .map(|p| PointF {
                X: p.x as f32,
                Y: p.y as f32,
            })
            .collect();
        GdipFillPolygon(g.0, b.0.cast(), points.as_ptr(), count, FillModeWinding);
    }
}

/// Draws a single vertically centered line with end ellipsis. Size is logical
/// pixel em height; weight >= 600 selects the embedded Bold face.
/// # Safety
/// `dc` must be a valid HDC exclusively used during this call.
#[allow(clippy::too_many_arguments)]
pub unsafe fn text(dc: HDC, r: RECT, value: &str, color: u32, size: i32, weight: i32, right: bool) {
    unsafe {
        draw_text(dc, r, value, color, size, weight, right, None);
    }
}
/// Draws a moving highlight clipped to the actual font glyphs.
/// # Safety
/// The HDC must be valid and exclusively used during this call.
#[allow(clippy::too_many_arguments)]
pub unsafe fn charging_text(
    dc: HDC,
    r: RECT,
    value: &str,
    color: u32,
    size: i32,
    weight: i32,
    phase: f32,
    highlight: u32,
) {
    unsafe {
        draw_text(
            dc,
            r,
            value,
            color,
            size,
            weight,
            false,
            Some((phase, highlight)),
        );
    }
}
#[allow(clippy::too_many_arguments)]
unsafe fn draw_text(
    dc: HDC,
    r: RECT,
    value: &str,
    color: u32,
    size: i32,
    weight: i32,
    right: bool,
    shimmer: Option<(f32, u32)>,
) {
    unsafe {
        if size <= 0 || value.is_empty() {
            return;
        }
        let Some(r) = bounds(r) else {
            return;
        };
        let Some(g) = Graphics::new(dc) else {
            return;
        };
        let Some(mut b) = brush(color) else {
            return;
        };
        if let Some((phase, shine)) = shimmer {
            let start = PointF {
                X: r.X - 60.,
                Y: r.Y,
            };
            let end = PointF {
                X: r.X + r.Width + 60.,
                Y: r.Y,
            };
            let mut gradient = null_mut();
            if GdipCreateLineBrush(
                &start,
                &end,
                argb(color),
                argb(color),
                WrapModeTileFlipX,
                &mut gradient,
            ) == Ok
            {
                let center = 0.06 + phase.clamp(0., 1.) * 0.88;
                let colors = [
                    argb(color),
                    argb(color),
                    argb(shine),
                    argb(color),
                    argb(color),
                ];
                let positions = [0., center - 0.045, center, center + 0.045, 1.];
                if GdipSetLinePresetBlend(gradient, colors.as_ptr(), positions.as_ptr(), 5) == Ok {
                    b = Brush(gradient.cast());
                } else {
                    GdipDeleteBrush(gradient.cast());
                }
            }
        }
        let fonts = fonts().lock().unwrap_or_else(|e| e.into_inner());
        if fonts.family == 0 {
            return;
        }
        let mut f = null_mut();
        if GdipCreateFont(
            fonts.family as *const GpFontFamily,
            size as f32,
            if weight >= 600 {
                FontStyleBold
            } else {
                FontStyleRegular
            },
            UnitPixel,
            &mut f,
        ) != Ok
        {
            return;
        }
        let f = Font(f);
        let mut format = null_mut();
        if GdipCreateStringFormat(
            StringFormatFlagsNoWrap | StringFormatFlagsNoFontFallback,
            0,
            &mut format,
        ) != Ok
        {
            return;
        }
        let format = Format(format);
        GdipSetStringFormatAlign(
            format.0,
            if right {
                StringAlignmentFar
            } else {
                StringAlignmentNear
            },
        );
        GdipSetStringFormatLineAlign(format.0, StringAlignmentCenter);
        GdipSetStringFormatTrimming(format.0, StringTrimmingEllipsisCharacter);
        let utf16: Vec<u16> = value.encode_utf16().collect();
        let Result::Ok(len) = i32::try_from(utf16.len()) else {
            return;
        };
        GdipDrawString(g.0, utf16.as_ptr(), len, f.0, &r, format.0, b.0.cast());
    }
}

/// Draws tightly packed, top-down straight-alpha BGRA32 (opaque cached glows
/// should carry alpha 255). Uses bicubic interpolation directly at target scale.
/// # Safety
/// `dc` must be a valid HDC exclusively used during this call.
pub unsafe fn image(dc: HDC, r: RECT, pixel_width: i32, pixel_height: i32, bgra: &[u8]) {
    unsafe {
        let Some(r) = bounds(r) else {
            return;
        };
        if pixel_width <= 0 || pixel_height <= 0 {
            return;
        }
        let Some(stride) = pixel_width.checked_mul(4) else {
            return;
        };
        let Some(len) = (stride as usize).checked_mul(pixel_height as usize) else {
            return;
        };
        if bgra.len() < len {
            return;
        }
        let Some(g) = Graphics::new(dc) else {
            return;
        };
        let mut bitmap = null_mut();
        // PixelFormat32bppARGB = Canonical | Alpha | (32 << 8) | 10.
        const BGRA32: i32 = 0x0026_200a;
        if GdipCreateBitmapFromScan0(
            pixel_width,
            pixel_height,
            stride,
            BGRA32,
            bgra.as_ptr(),
            &mut bitmap,
        ) != Ok
        {
            return;
        }
        let bitmap = Bitmap(bitmap);
        let mut attrs = null_mut();
        if GdipCreateImageAttributes(&mut attrs) != Ok {
            return;
        }
        let attrs = Attributes(attrs);
        // Mirror edge samples to prevent translucent/dark bicubic borders.
        GdipSetImageAttributesWrapMode(attrs.0, WrapModeTileFlipXY, 0, 0);
        GdipSetInterpolationMode(g.0, InterpolationModeHighQualityBicubic);
        GdipSetPixelOffsetMode(g.0, PixelOffsetModeHalf);
        GdipDrawImageRectRect(
            g.0,
            bitmap.0.cast(),
            r.X,
            r.Y,
            r.Width,
            r.Height,
            0.0,
            0.0,
            pixel_width as f32,
            pixel_height as f32,
            UnitPixel,
            attrs.0,
            0,
            null_mut(),
        );
    }
}

/// Draws an antialiased polyline in logical coordinates.
/// # Safety
/// The HDC must be valid and exclusively used during this call.
pub unsafe fn line(dc: HDC, points: &[POINT], color: u32, width: f32) {
    unsafe {
        if points.len() < 2 {
            return;
        }
        let Some(g) = Graphics::new(dc) else {
            return;
        };
        let mut pen = null_mut();
        if GdipCreatePen1(argb(color), width, UnitWorld, &mut pen) != Ok {
            return;
        }
        let points: Vec<PointF> = points
            .iter()
            .map(|p| PointF {
                X: p.x as f32,
                Y: p.y as f32,
            })
            .collect();
        GdipDrawLines(g.0, pen, points.as_ptr(), points.len() as i32);
        GdipDeletePen(pen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Graphics::Gdi::*;

    struct Surface {
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
        bits: *mut u32,
        size: i32,
    }
    impl Surface {
        unsafe fn new(size: i32) -> Self {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let mut info: BITMAPINFO = zeroed();
                info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
                info.bmiHeader.biWidth = size;
                info.bmiHeader.biHeight = -size;
                info.bmiHeader.biPlanes = 1;
                info.bmiHeader.biBitCount = 32;
                info.bmiHeader.biCompression = BI_RGB;
                let mut bits = null_mut();
                let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                assert!(!bitmap.is_null());
                let old = SelectObject(dc, bitmap);
                Self {
                    dc,
                    bitmap,
                    old,
                    bits: bits.cast(),
                    size,
                }
            }
        }
    }
    impl Drop for Surface {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.old);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }

    #[test]
    fn embedded_clear_sans_is_really_available() {
        assert!(font_ready(), "{}", font_diagnostics());
        assert_eq!(font_name(), "Clear Sans");
    }
    #[test]
    fn rounded_edges_are_antialiased_at_native_and_double_resolution() {
        unsafe {
            for scale in [1, 2] {
                let s = Surface::new(32 * scale);
                let _target = target(s.dc, s.size, s.size, 32, 32);
                let r = RECT {
                    left: 0,
                    top: 0,
                    right: 32,
                    bottom: 32,
                };
                fill(s.dc, r, 0);
                rounded(s.dc, r, 0xffffff, 32);
                GdiFlush();
                let pixels = std::slice::from_raw_parts(s.bits, (s.size * s.size) as usize);
                let blended = pixels
                    .iter()
                    .filter(|p| **p & 0xffffff != 0 && **p & 0xffffff != 0xffffff)
                    .count();
                assert!(
                    blended > 16,
                    "Expected blended edge coverage at {scale}x, got {blended}"
                );
                assert_eq!(
                    pixels[(s.size / 2 * s.size + s.size / 2) as usize] & 0xffffff,
                    0xffffff
                );
                assert_eq!(pixels[0] & 0xffffff, 0);
            }
        }
    }
    #[test]
    fn logical_translation_matches_scaled_pixels_and_restores_target() {
        unsafe {
            for scale in [1, 2] {
                let s = Surface::new(64 * scale);
                let _target = target(s.dc, s.size, s.size, 64, 64);
                fill(
                    s.dc,
                    RECT {
                        left: 0,
                        top: 0,
                        right: 64,
                        bottom: 64,
                    },
                    0,
                );
                {
                    let _offset = translate(s.dc, -10.0);
                    fill(
                        s.dc,
                        RECT {
                            left: 20,
                            top: 10,
                            right: 30,
                            bottom: 20,
                        },
                        0xffffff,
                    );
                }
                fill(
                    s.dc,
                    RECT {
                        left: 40,
                        top: 10,
                        right: 50,
                        bottom: 20,
                    },
                    0xffffff,
                );
                GdiFlush();
                let pixels = std::slice::from_raw_parts(s.bits, (s.size * s.size) as usize);
                let pixel =
                    |x: i32, y: i32| pixels[(y * scale * s.size + x * scale) as usize] & 0xffffff;
                assert_eq!(pixel(15, 15), 0xffffff);
                assert_eq!(pixel(25, 15), 0);
                assert_eq!(pixel(45, 15), 0xffffff);
            }
        }
    }
    #[test]
    fn clearing_a_control_surface_leaves_no_dark_edge_seams() {
        unsafe {
            for scale in [1, 2] {
                let surface = Surface::new(32 * scale);
                let _target = target(surface.dc, surface.size, surface.size, 32, 32);
                clear(surface.dc, 0xffffff);
                GdiFlush();
                let pixels = std::slice::from_raw_parts(
                    surface.bits,
                    (surface.size * surface.size) as usize,
                );
                assert!(pixels.iter().all(|pixel| pixel & 0xffffff == 0xffffff));
            }
        }
    }
    #[test]
    fn charging_shimmer_moves_without_painting_outside_glyphs() {
        unsafe {
            let surface = Surface::new(240);
            let r = RECT {
                left: 5,
                top: 10,
                right: 235,
                bottom: 110,
            };
            let bounds = RECT {
                left: 0,
                top: 0,
                right: 240,
                bottom: 240,
            };
            fill(surface.dc, bounds, 0);
            text(surface.dc, r, "75%", 0xffffff, 76, 600, false);
            GdiFlush();
            let glyphs = std::slice::from_raw_parts(surface.bits, 240 * 240).to_vec();
            let mut frames = Vec::new();
            for phase in [0.2, 0.6] {
                fill(surface.dc, bounds, 0);
                charging_text(surface.dc, r, "75%", 0x50ae60, 76, 600, phase, 0xd0eac6);
                GdiFlush();
                let pixels = std::slice::from_raw_parts(surface.bits, 240 * 240).to_vec();
                for (glyph, pixel) in glyphs.iter().zip(&pixels) {
                    if glyph & 0xffffff == 0 {
                        assert_eq!(pixel & 0xffffff, 0);
                    }
                }
                frames.push(pixels);
            }
            assert!(
                frames[0]
                    .iter()
                    .zip(&frames[1])
                    .filter(|(a, b)| (**a & 0xffffff) != (**b & 0xffffff))
                    .count()
                    > 50
            );
        }
    }
}
