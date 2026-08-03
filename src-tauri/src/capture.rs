//! Chụp vùng màn hình -> RGBA bytes (thay mss của bản Python) + tô trắng vùng loại trừ.

use std::cell::RefCell;

use crate::config::Region;

thread_local! {
    /// (điểm đã tra, gốc màn hình, monitor). `Monitor::from_point` liệt kê lại toàn bộ màn hình
    /// mỗi lần gọi — vô ích khi vùng quét đứng yên hàng giờ.
    /// ponytail: khoá theo điểm góc trên-trái; sắp xếp lại màn hình mà vùng quét không đổi thì
    /// cache vẫn cũ tới lần đổi vùng kế tiếp. Đổi vùng là bấm "SỬA VÙNG QUÉT", đủ để thoát.
    static MONITOR: RefCell<Option<((i32, i32), (i32, i32), xcap::Monitor)>> = const { RefCell::new(None) };
}

/// Chụp theo toạ độ màn hình tuyệt đối. Trả (rgba, width, height) đúng thứ tự xcap.
/// OneOCR nhận thẳng RGBA; nhánh Windows OCR tự hoán R<->B (xem [`crate::ocr`]).
pub fn grab_rgba(region: &Region) -> Result<(Vec<u8>, u32, u32), String> {
    let point = (region.left, region.top);
    MONITOR.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !matches!(&*cache, Some((p, _, _)) if *p == point) {
            let mon = xcap::Monitor::from_point(point.0, point.1).map_err(|e| e.to_string())?;
            let origin = (
                mon.x().map_err(|e| e.to_string())?,
                mon.y().map_err(|e| e.to_string())?,
            );
            *cache = Some((point, origin, mon));
        }
        let (_, (mx, my), mon) = cache.as_ref().expect("vừa gán ở trên");
        let img = mon
            .capture_region(
                (region.left - mx).max(0) as u32,
                (region.top - my).max(0) as u32,
                region.width,
                region.height,
            )
            .map_err(|e| e.to_string())?;
        let (w, h) = (img.width(), img.height());
        Ok((img.into_raw(), w, h))
    })
}

/// Tô trắng vùng loại trừ (toạ độ màn hình) trong ảnh khung quét. Port mask_exclusion của narrator.py.
/// Trắng là 255 ở mọi kênh nên hàm này không quan tâm RGBA hay BGRA.
pub fn mask_exclusion(bgra: &mut [u8], w: u32, h: u32, scan: &Region, exclude: Option<&Region>) {
    let Some(ex) = exclude else { return };
    if bgra.len() != (w as usize) * (h as usize) * 4 {
        return;
    }
    let (w, h) = (w as i64, h as i64);
    let x0 = (ex.left - scan.left) as i64;
    let y0 = (ex.top - scan.top) as i64;
    let x1 = (x0 + ex.width as i64).clamp(0, w);
    let y1 = (y0 + ex.height as i64).clamp(0, h);
    let x0 = x0.clamp(0, w);
    let y0 = y0.clamp(0, h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for y in y0..y1 {
        let row = (y * w) as usize * 4;
        bgra[row + x0 as usize * 4..row + x1 as usize * 4].fill(255);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, width: u32, height: u32) -> Region {
        Region { left, top, width, height }
    }

    #[test]
    fn to_trang_dung_o_vung_loai_tru() {
        let (w, h) = (4u32, 3u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let scan = r(100, 100, w, h);
        // loại trừ ô (1,1) rộng 2 cao 1 -> pixel (1,1) và (2,1)
        mask_exclusion(&mut buf, w, h, &scan, Some(&r(101, 101, 2, 1)));
        let px = |x: u32, y: u32| buf[((y * w + x) * 4) as usize];
        assert_eq!((px(1, 1), px(2, 1)), (255, 255));
        assert_eq!((px(0, 1), px(3, 1), px(1, 0), px(1, 2)), (0, 0, 0, 0));
    }

    #[test]
    fn vung_loai_tru_ngoai_khung_khong_lam_gi() {
        let (w, h) = (4u32, 3u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let scan = r(100, 100, w, h);
        mask_exclusion(&mut buf, w, h, &scan, Some(&r(500, 500, 10, 10)));
        assert!(buf.iter().all(|&b| b == 0));
        mask_exclusion(&mut buf, w, h, &scan, None);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn vung_loai_tru_cat_bien_thi_bi_ket() {
        let (w, h) = (4u32, 3u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let scan = r(100, 100, w, h);
        // bắt đầu trước khung, tràn ra ngoài -> chỉ tô phần giao
        mask_exclusion(&mut buf, w, h, &scan, Some(&r(98, 100, 100, 1)));
        let px = |x: u32, y: u32| buf[((y * w + x) * 4) as usize];
        assert_eq!((px(0, 0), px(3, 0)), (255, 255));
        assert_eq!(px(0, 1), 0);
    }
}
