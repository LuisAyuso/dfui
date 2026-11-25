use dfui::taffy::{Dimension, Size, Style};
use dfui::widgets::Styled;
use dfui::{Color, Element, PaintCx, Rect, Widget};

use crate::Player;

/// A [`Player`]'s picture, fitted into the widget's box over black.
pub struct Video {
    player: Player,
    style: Style,
}

/// Fills its parent unless sized otherwise.
pub fn video(player: &Player) -> Video {
    Video {
        player: player.clone(),
        style: Style {
            size: Size {
                width: Dimension::percent(1.0),
                height: Dimension::percent(1.0),
            },
            ..Style::default()
        },
    }
}

impl From<Video> for Element {
    fn from(w: Video) -> Self {
        Element::new(w)
    }
}

impl Styled for Video {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Video {
    fn style(&self) -> Style {
        self.style.clone()
    }

    fn paint(&self, cx: &mut PaintCx) {
        let r = cx.rect;
        cx.fill(r, 0.0, Color::BLACK);
        let frame = self.player.frame();
        if let Some(f) = &frame {
            let (w, h) = if r.w / r.h.max(1.0) > f.aspect {
                (r.h * f.aspect, r.h)
            } else {
                (r.w, r.w / f.aspect)
            };
            let fit = Rect::new(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h);
            cx.image(fit, &f.bitmap, 0.0, Color::WHITE);
        }
        if !self.player.ended() && (!self.player.paused() || frame.is_none()) {
            cx.request_frame();
        }
    }
}
