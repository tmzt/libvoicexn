//! The screen: a libcontrols `Router` that keeps one `Label` node per line
//! for the life of the app. A change of text, colour or size edits the
//! nodes; the draw list is rebuilt only when something changed, and a
//! frame with no change draws the list it already has.

use libcontrols::{DrawContext, Label, NodeId, Rect, Router, TextAlign};
use libmsdf::drawlist::DrawList;

/// Lines of body text the screen can show.
const LINES: usize = 16;

pub const TEXT: [f32; 4] = [0.92, 0.93, 0.95, 1.0];
pub const MUTED: [f32; 4] = [0.60, 0.62, 0.68, 1.0];
pub const SPOKEN: [f32; 4] = [0.55, 0.78, 1.0, 1.0];

/// One line of body text, before layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    /// In points; scaled by the display's scale factor.
    pub size: f32,
    pub color: [f32; 4],
    /// Space before the line, in points.
    pub gap: f32,
}

/// Splits `text` into lines of at most `max` characters at spaces.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The persistent screen.
pub struct Screen {
    router: Router,
    title: NodeId,
    status: [NodeId; 2],
    lines: Vec<NodeId>,
    /// What the screen shows, as last laid out.
    shown: (Vec<String>, Vec<Line>, Layout),
    dirty: bool,
}

/// Surface size, scale factor and safe area top and bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
    pub scale: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Layout {
    const PAD: f32 = 20.0;

    /// How many characters of `size` points fit on a line: roughly half an
    /// em each in Roboto.
    pub fn columns(&self, size: f32) -> usize {
        ((self.w - 2.0 * Self::PAD * self.scale) / (size * self.scale * 0.5)).max(8.0) as usize
    }
}

impl Screen {
    pub fn new() -> Self {
        let mut router = Router::new();
        let label = |text: &str, size: f32, color, align| {
            Label::new(text, Rect::new(0.0, 0.0, 1.0, 1.0))
                .with_font_size(size)
                .with_color(color)
                .with_align(align)
        };
        let title = router.add(None, label("voicexn", 32.0, TEXT, TextAlign::Center));
        let status = [
            router.add(None, label("", 15.0, MUTED, TextAlign::Center)),
            router.add(None, label("", 15.0, MUTED, TextAlign::Center)),
        ];
        let lines = (0..LINES)
            .map(|_| router.add(None, label("", 20.0, TEXT, TextAlign::Left)))
            .collect();
        Self {
            router,
            title,
            status,
            lines,
            shown: Default::default(),
            dirty: true,
        }
    }

    /// Sets what the screen shows; nodes change only if it differs.
    pub fn set(&mut self, layout: Layout, status: Vec<String>, body: Vec<Line>) {
        if (&self.shown.0, &self.shown.1, self.shown.2) == (&status, &body, layout) {
            return;
        }
        let s = layout.scale;
        let pad = Layout::PAD * s;
        let mut y = layout.top + 24.0 * s;
        let place =
            |router: &mut Router, y: &mut f32, id, text: Option<&str>, size: f32, color, x: f32| {
                let label = router.get_mut::<Label>(id).expect("a label node");
                if let Some(text) = text {
                    label.set_text(text);
                }
                label.font_size = Some(size * s);
                label.color = Some(color);
                label.rect = Rect::new(x, *y, layout.w - 2.0 * x, size * s * 1.3);
                *y += size * s * 1.35;
            };
        place(&mut self.router, &mut y, self.title, None, 32.0, TEXT, 0.0);
        for (i, &id) in self.status.iter().enumerate() {
            let text = status.get(i).map_or("", String::as_str);
            place(&mut self.router, &mut y, id, Some(text), 15.0, MUTED, 0.0);
        }
        let bottom = layout.h - layout.bottom - 24.0 * s;
        let mut body_lines = body.iter();
        for &id in &self.lines {
            match body_lines.next() {
                Some(line) if y + line.gap * s + line.size * s <= bottom => {
                    y += line.gap * s;
                    place(
                        &mut self.router,
                        &mut y,
                        id,
                        Some(&line.text),
                        line.size,
                        line.color,
                        pad,
                    );
                }
                _ => {
                    let label = self.router.get_mut::<Label>(id).expect("a label node");
                    label.set_text("");
                }
            }
        }
        self.shown = (status, body, layout);
        self.dirty = true;
    }

    /// Rebuilds `list` if the screen changed since the last call. Returns
    /// whether it did.
    pub fn draw(&mut self, list: &mut DrawList, ctx: &DrawContext) -> bool {
        if !self.dirty {
            return false;
        }
        list.clear();
        self.router.render(list, ctx);
        self.dirty = false;
        true
    }
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_at_spaces() {
        assert_eq!(wrap("one two three four", 9), ["one two", "three", "four"]);
        assert_eq!(wrap("  ", 9), Vec::<String>::new());
        assert_eq!(wrap("unbreakable", 4), ["unbreakable"]);
    }

    #[test]
    fn the_list_is_rebuilt_only_on_change() {
        let mut screen = Screen::new();
        let layout = Layout {
            w: 1080.0,
            h: 2340.0,
            scale: 2.0,
            top: 80.0,
            bottom: 40.0,
        };
        let body = vec![Line {
            text: "hello".into(),
            size: 20.0,
            color: TEXT,
            gap: 0.0,
        }];
        let mut list = DrawList::new();
        let ctx = DrawContext::default();
        screen.set(layout, vec!["ready".into()], body.clone());
        assert!(screen.draw(&mut list, &ctx));
        assert!(!screen.draw(&mut list, &ctx));
        screen.set(layout, vec!["ready".into()], body.clone());
        assert!(
            !screen.draw(&mut list, &ctx),
            "the same content is no change"
        );
        screen.set(layout, vec!["listening".into()], body);
        assert!(screen.draw(&mut list, &ctx));
        let status = screen.router.get::<Label>(screen.status[0]).unwrap();
        assert_eq!(status.text(), "listening");
    }
}
