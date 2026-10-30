//! Optional-content filtering during PDF interpretation.

use hayro_interpret::font::GlyphRun;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;
use hayro_syntax::object::Dict;
use kurbo::{Affine, BezPath, Point, Rect};
use std::fmt::Write;

const PROPERTIES: &str = "/Properties << /Hidden 5 0 R /Visible 6 0 R
    /Membership << /Type /OCMD /OCGs [5 0 R] /P /AllOff >>
    /Regular << /MCID 1 >> >>";

fn stream(dict: &str, contents: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{contents}\nendstream",
        contents.len()
    )
}

fn document(contents: &str, resources: &str, extra_objects: &[String]) -> Pdf {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R 6 0 R]
            /D << /OFF [5 0 R] >> >> >>"
            .to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100]
            /Resources << {PROPERTIES} {resources} >> /Contents 4 0 R >>"
        ),
        stream("", contents),
        "<< /Type /OCG /Name (Hidden) >>".to_string(),
        "<< /Type /OCG /Name (Visible) >>".to_string(),
    ];
    objects.extend_from_slice(extra_objects);

    let mut data = "%PDF-1.7\n".to_string();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(data.len());
        writeln!(data, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
    }
    let xref = data.len();
    let size = objects.len() + 1;
    writeln!(data, "xref\n0 {size}\n0000000000 65535 f ").unwrap();
    for offset in offsets {
        writeln!(data, "{offset:010} 00000 n ").unwrap();
    }
    writeln!(
        data,
        "trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
    )
    .unwrap();
    Pdf::new(data.into_bytes()).unwrap()
}

#[derive(Debug, PartialEq)]
enum Event {
    Path,
    Text,
    Image,
}

#[derive(Default)]
struct Recorder {
    events: Vec<Event>,
    glyph_positions: Vec<f64>,
    clips: usize,
    clip_depth: usize,
    marked_depth: usize,
}

impl Device<'_> for Recorder {
    fn begin_marked_content(&mut self, _: &[u8], _: Option<&Dict<'_>>) {
        self.marked_depth += 1;
    }

    fn end_marked_content(&mut self) {
        self.marked_depth -= 1;
    }

    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'_>, _: &DrawMode) {
        self.events.push(Event::Path);
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, '_>, props: DrawProps<'_>, _: &DrawMode) {
        self.events.push(Event::Text);
        for glyph in run.glyphs() {
            let origin = props.transform * glyph.transform() * Point::ORIGIN;
            self.glyph_positions.push(origin.x);
        }
    }

    fn draw_image(&mut self, _: Image<'_, '_>, _: ImageDrawProps<'_>) {
        self.events.push(Event::Image);
    }

    fn push_clip_path(&mut self, _: &ClipPath) {
        self.clips += 1;
        self.clip_depth += 1;
    }

    fn pop_clip(&mut self) {
        self.clip_depth -= 1;
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn pop_transparency_group(&mut self) {}
}

fn run(pdf: &Pdf) -> Recorder {
    let mut recorder = Recorder::default();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        &InterpreterCache::new(),
        pdf.xref(),
        InterpreterSettings::default(),
    );
    interpret_page(&pdf.pages()[0], &mut context, &mut recorder);
    assert_eq!(recorder.clip_depth, 0);
    assert_eq!(recorder.marked_depth, 0);
    recorder
}

#[test]
fn a_visible_layer_inside_a_hidden_one_stays_hidden() {
    let pdf = document(
        "/OC /Hidden BDC /Span BMC /OC /Visible BDC 0 0 10 10 re f EMC EMC EMC",
        "",
        &[],
    );
    assert_eq!(run(&pdf).events, []);
}

#[test]
fn only_oc_tags_name_optional_content() {
    let pdf = document(
        "/OC /Membership BDC 0 0 10 10 re f EMC
         /Span /Hidden BDC 0 0 10 10 re f EMC
         /OC /Regular BDC 0 0 10 10 re f EMC
         /OC /Hidden BDC EMC /OC /Hidden BDC EMC",
        "",
        &[],
    );
    assert_eq!(run(&pdf).events, [Event::Path, Event::Path, Event::Path]);
}

#[test]
fn hidden_form_and_image_are_skipped() {
    let pdf = document(
        "/Form Do /Image Do",
        "/XObject << /Form 7 0 R /Image 8 0 R >>",
        &[
            stream(
                "/Subtype /Form /BBox [0 0 10 10] /OC 5 0 R",
                "/OC /Visible BDC 0 0 10 10 re f EMC",
            ),
            stream(
                "/Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray
                /BitsPerComponent 8 /OC 5 0 R",
                "x",
            ),
        ],
    );
    assert_eq!(run(&pdf).events, []);
}

#[test]
fn marked_content_left_open_in_a_form_ends_with_the_form() {
    let pdf = document(
        "EMC /Form Do 0 0 10 10 re f",
        "/XObject << /Form 7 0 R >>",
        &[stream(
            "/Subtype /Form /BBox [0 0 10 10]",
            "EMC /OC /Hidden BDC /Span BMC 0 0 10 10 re f",
        )],
    );
    assert_eq!(run(&pdf).events, [Event::Path]);
}

#[test]
fn hidden_content_keeps_path_clipping() {
    let pdf = document(
        "/OC /Hidden BDC 0 0 10 10 re W n EMC 0 0 50 50 re f",
        "",
        &[],
    );
    let recorder = run(&pdf);
    assert_eq!(recorder.clips, 1);
    assert_eq!(recorder.events, [Event::Path]);
}

#[test]
#[cfg(feature = "embed-fonts")]
fn hidden_text_keeps_advance_and_clipping() {
    let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string();
    let resources = "/Font << /F1 7 0 R >>";
    let pdf = document(
        "BT /F1 10 Tf /OC /Hidden BDC (A) Tj EMC (B) Tj ET",
        resources,
        std::slice::from_ref(&font),
    );
    let recorder = run(&pdf);
    assert_eq!(recorder.events, [Event::Text]);
    assert!(recorder.glyph_positions[0] > 0.0);

    let pdf = document(
        "BT /F1 10 Tf /OC /Hidden BDC 4 Tr (A) Tj EMC ET 0 0 50 50 re f",
        resources,
        &[font],
    );
    let recorder = run(&pdf);
    assert_eq!(recorder.clips, 1);
    assert_eq!(recorder.events, [Event::Path]);
}
