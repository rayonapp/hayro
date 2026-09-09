//! Optional-content callbacks and filtering during PDF interpretation.

use hayro_interpret::font::GlyphRun;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;
use hayro_syntax::object::{Dict, ObjectIdentifier};
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
    Begin(Option<i32>, bool),
    End,
    Path,
    Text,
    Image,
}

#[derive(Default)]
struct Recorder {
    include_hidden: bool,
    reject: Option<i32>,
    events: Vec<Event>,
    names: Vec<Vec<u8>>,
    glyph_positions: Vec<f64>,
    clips: usize,
    clip_depth: usize,
    marked_depth: usize,
}

impl Device<'_> for Recorder {
    fn begin_optional_content(
        &mut self,
        id: Option<ObjectIdentifier>,
        properties: &Dict<'_>,
        visible: bool,
    ) -> bool {
        let id = id.map(|id| id.obj_number);
        self.events.push(Event::Begin(id, visible));
        if let Some(name) = properties.get::<hayro_syntax::object::String<'_>>(b"Name") {
            self.names.push(name.as_bytes().to_vec());
        }
        !self.reject.is_some_and(|rejected| Some(rejected) == id)
            && (self.include_hidden || visible)
    }

    fn end_optional_content(&mut self) {
        self.events.push(Event::End);
    }

    fn begin_marked_content(&mut self, _: &[u8], _: Option<i32>) {
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

fn run(pdf: &Pdf, recorder: &mut Recorder) {
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        &InterpreterCache::new(),
        pdf.xref(),
        InterpreterSettings::default(),
    );
    interpret_page(&pdf.pages()[0], &mut context, recorder);
    assert_eq!(recorder.clip_depth, 0);
    assert_eq!(recorder.marked_depth, 0);
}

#[test]
fn nested_layers_keep_local_visibility() {
    let pdf = document(
        "/OC /Hidden BDC /Span BMC /OC /Visible BDC 0 0 10 10 re f EMC EMC EMC",
        "",
        &[],
    );
    for include_hidden in [false, true] {
        let mut recorder = Recorder {
            include_hidden,
            ..Default::default()
        };
        run(&pdf, &mut recorder);
        let mut expected = vec![Event::Begin(Some(5), false), Event::Begin(Some(6), true)];
        if include_hidden {
            expected.push(Event::Path);
        }
        expected.extend([Event::End, Event::End]);
        assert_eq!(recorder.events, expected);
        assert_eq!(recorder.names, [b"Hidden".to_vec(), b"Visible".to_vec()]);
    }

    let mut recorder = Recorder {
        include_hidden: true,
        reject: Some(5),
        ..Default::default()
    };
    run(&pdf, &mut recorder);
    assert!(!recorder.events.contains(&Event::Path));
}

#[test]
fn membership_and_ordinary_marked_content() {
    let pdf = document(
        "/OC /Membership BDC 0 0 10 10 re f EMC
         /Span /Hidden BDC 0 0 10 10 re f EMC
         /OC /Regular BDC 0 0 10 10 re f EMC
         /OC /Hidden BDC EMC /OC /Hidden BDC EMC",
        "",
        &[],
    );
    for include_hidden in [false, true] {
        let mut recorder = Recorder {
            include_hidden,
            ..Default::default()
        };
        run(&pdf, &mut recorder);
        assert_eq!(
            recorder.events,
            [
                Event::Begin(None, true),
                Event::Path,
                Event::End,
                Event::Path,
                Event::Path,
                Event::Begin(Some(5), false),
                Event::End,
                Event::Begin(Some(5), false),
                Event::End,
            ]
        );
    }
}

#[test]
fn hidden_form_and_image() {
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
    let mut recorder = Recorder::default();
    run(&pdf, &mut recorder);
    assert_eq!(
        recorder.events,
        [
            Event::Begin(Some(5), false),
            Event::End,
            Event::Begin(Some(5), false),
            Event::End,
        ]
    );

    let mut recorder = Recorder {
        include_hidden: true,
        ..Default::default()
    };
    run(&pdf, &mut recorder);
    assert_eq!(
        recorder.events,
        [
            Event::Begin(Some(5), false),
            Event::Begin(Some(6), true),
            Event::Path,
            Event::End,
            Event::End,
            Event::Begin(Some(5), false),
            Event::Image,
            Event::End,
        ]
    );
}

#[test]
fn malformed_form_keeps_enclosing_scope() {
    let pdf = document(
        "EMC /OC /Hidden BDC /Form Do 0 0 10 10 re f",
        "/XObject << /Form 7 0 R >>",
        &[stream(
            "/Subtype /Form /BBox [0 0 10 10]",
            "EMC /OC /Visible BDC /Span BMC 0 0 10 10 re f",
        )],
    );
    let mut recorder = Recorder {
        include_hidden: true,
        ..Default::default()
    };
    run(&pdf, &mut recorder);
    assert_eq!(
        recorder.events,
        [
            Event::Begin(Some(5), false),
            Event::Begin(Some(6), true),
            Event::Path,
            Event::End,
            Event::Path,
            Event::End,
        ]
    );
}

#[test]
fn hidden_content_keeps_path_clipping() {
    let pdf = document(
        "/OC /Hidden BDC 0 0 10 10 re W n EMC 0 0 50 50 re f",
        "",
        &[],
    );
    let mut recorder = Recorder::default();
    run(&pdf, &mut recorder);
    assert_eq!(recorder.clips, 1);
    assert_eq!(
        recorder.events,
        [Event::Begin(Some(5), false), Event::End, Event::Path]
    );
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
    let mut visible = Recorder::default();
    let mut all = Recorder {
        include_hidden: true,
        ..Default::default()
    };
    run(&pdf, &mut visible);
    run(&pdf, &mut all);
    assert_eq!(all.glyph_positions.len(), 2);
    assert_eq!(visible.glyph_positions, all.glyph_positions[1..]);

    let pdf = document(
        "BT /F1 10 Tf /OC /Hidden BDC 4 Tr (A) Tj EMC ET 0 0 50 50 re f",
        resources,
        &[font],
    );
    let mut recorder = Recorder::default();
    run(&pdf, &mut recorder);
    assert_eq!(recorder.clips, 1);
    assert_eq!(
        recorder.events,
        [Event::Begin(Some(5), false), Event::End, Event::Path]
    );
}
