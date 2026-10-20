//! Icons via `IShellItemImageFactory`: the same images Start and Explorer show.

use std::ffi::c_void;
use std::mem::size_of;

use windows::core::HSTRING;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};

use crate::icons::bgra_premultiplied_to_rgba;
use crate::platform::Image;

/// Must run on an STA thread. `parsing_name` is a path or shell path;
/// `size` is in pixels (square).
pub fn icon(parsing_name: &HSTRING, size: u32) -> windows::core::Result<Image> {
    let side = i32::try_from(size).unwrap_or(64);
    // SAFETY: COM calls on an STA thread; the bitmap is deleted after reading.
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(parsing_name, None::<&IBindCtx>)?;
        let bitmap = factory.GetImage(
            SIZE { cx: side, cy: side },
            SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
        )?;
        let image = read_bitmap(bitmap);
        let _ = DeleteObject(bitmap.into());
        image
    }
}

/// Copies a 32-bit bitmap out as top-down BGRA, then converts to RGBA.
unsafe fn read_bitmap(bitmap: HBITMAP) -> windows::core::Result<Image> {
    // SAFETY: plain GDI calls with buffers sized from the bitmap's own header.
    unsafe {
        let mut info = BITMAP::default();
        let got = GetObjectW(
            bitmap.into(),
            size_of::<BITMAP>() as i32,
            Some(&mut info as *mut BITMAP as *mut c_void),
        );
        if got == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let (width, height) = (info.bmWidth, info.bmHeight.abs());

        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative height asks for top-down rows.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(bgra.as_mut_ptr().cast()),
            &mut header,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        if lines == 0 {
            return Err(windows::core::Error::from_thread());
        }
        Ok(Image {
            width: width as u32,
            height: height as u32,
            rgba: bgra_premultiplied_to_rgba(&bgra),
        })
    }
}
