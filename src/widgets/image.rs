use taffy::{AvailableSpace, Size, Style};

use super::Styled;
use crate::render::{Bitmap, Color};
use crate::text::Text;
use crate::widget::{Element, PaintCx, Widget};

pub struct Image {
    bitmap: Bitmap,
    radius: f32,
    tint: Color,
    style: Style,
}

/// `bitmap` stretched over the widget's box. Its natural size is one logical
/// pixel per bitmap pixel, and a box sized on one axis keeps the aspect ratio.
pub fn image(bitmap: Bitmap) -> Image {
    let aspect = bitmap.width() as f32 / bitmap.height().max(1) as f32;
    Image {
        bitmap,
        radius: 0.0,
        tint: Color::WHITE,
        style: Style {
            aspect_ratio: Some(aspect),
            ..Style::default()
        },
    }
}

impl Image {
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    /// Multiplies every pixel; `Color::WHITE.alpha(a)` fades the image.
    pub fn tint(mut self, c: Color) -> Self {
        self.tint = c;
        self
    }
}

impl From<Image> for Element {
    fn from(w: Image) -> Self {
        Element::new(w)
    }
}

impl Styled for Image {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Image {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn measure(
        &self,
        known: Size<Option<f32>>,
        _available: Size<AvailableSpace>,
        _text: &Text,
    ) -> Size<f32> {
        let (w, h) = (self.bitmap.width() as f32, self.bitmap.height() as f32);
        match (known.width, known.height) {
            (Some(kw), Some(kh)) => Size {
                width: kw,
                height: kh,
            },
            (Some(kw), None) => Size {
                width: kw,
                height: kw * h / w.max(1.0),
            },
            (None, Some(kh)) => Size {
                width: kh * w / h.max(1.0),
                height: kh,
            },
            (None, None) => Size {
                width: w,
                height: h,
            },
        }
    }
    fn measure_key(&self) -> u64 {
        (self.bitmap.width() as u64) << 32 | self.bitmap.height() as u64
    }
    fn paint(&self, cx: &mut PaintCx) {
        cx.image(cx.rect, &self.bitmap, self.radius, self.tint);
    }
}
