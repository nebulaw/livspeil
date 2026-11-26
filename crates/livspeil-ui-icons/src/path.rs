// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Strict SVG path data parser.
//!
//! zeno has its own, but it silently drops a truncated last command and,
//! as of 0.3.3, picks the wrong first control point for repeated `s`/`t`
//! and for `s`/`t` after a non-curve, so path data is parsed here and only
//! rasterized by zeno.

use zeno::{Angle, ArcSize, ArcSweep, Command, PathBuilder, Point};

struct Scanner<'a> {
    s: &'a [u8],
    at: usize,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn skip_separators(&mut self) {
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_whitespace() || c == b',')
        {
            self.at += 1;
        }
    }

    fn digits(&mut self) -> bool {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        self.at > start
    }

    fn accept(&mut self, any: &[u8]) -> bool {
        let hit = self.peek().is_some_and(|c| any.contains(&c));
        self.at += usize::from(hit);
        hit
    }

    fn number(&mut self) -> Result<f32, String> {
        self.skip_separators();
        let start = self.at;
        self.accept(b"+-");
        let whole = self.digits();
        let fraction = self.accept(b".") && self.digits();
        if !whole && !fraction {
            return Err(format!("expected a number at byte {start}"));
        }
        if self.accept(b"eE") {
            self.accept(b"+-");
            if !self.digits() {
                return Err(format!("bad exponent at byte {start}"));
            }
        }
        std::str::from_utf8(&self.s[start..self.at])
            .ok()
            .and_then(|t| t.parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("bad number at byte {start}"))
    }

    /// An arc flag: one `0` or `1`, which need not be separated from what
    /// follows.
    fn flag(&mut self) -> Result<bool, String> {
        self.skip_separators();
        let flag = match self.peek() {
            Some(b'0') => false,
            Some(b'1') => true,
            _ => return Err(format!("expected an arc flag at byte {}", self.at)),
        };
        self.at += 1;
        Ok(flag)
    }

    fn point(&mut self, origin: Point) -> Result<Point, String> {
        let x = self.number()?;
        let y = self.number()?;
        Ok(Point::new(origin.x + x, origin.y + y))
    }
}

fn reflect(about: Point, control: Option<Point>) -> Point {
    control.map_or(about, |c| {
        Point::new(2.0 * about.x - c.x, 2.0 * about.y - c.y)
    })
}

/// The commands of path data `d`.
pub(crate) fn parse_path(d: &str) -> Result<Vec<Command>, String> {
    let mut s = Scanner {
        s: d.as_bytes(),
        at: 0,
    };
    let mut out: Vec<Command> = Vec::new();
    let zero = Point::new(0.0, 0.0);
    let (mut cur, mut start) = (zero, zero);
    // The last control point, if the previous command was a cubic (`cubic`)
    // or a quadratic (`quad`).
    let (mut cubic, mut quad): (Option<Point>, Option<Point>) = (None, None);
    let mut closed = false;
    loop {
        s.skip_separators();
        let Some(mut op) = s.peek() else { break };
        if !b"MmLlHhVvCcSsQqTtAaZz".contains(&op) {
            return Err(format!("unsupported path command at byte {}", s.at));
        }
        if out.is_empty() && !matches!(op, b'M' | b'm') {
            return Err("path data must start with a move".to_owned());
        }
        s.at += 1;
        if matches!(op, b'Z' | b'z') {
            out.push(Command::Close);
            cur = start;
            (cubic, quad, closed) = (None, None, true);
            continue;
        }
        // Drawing right after a close starts a new subpath where the closed
        // one started.
        if closed && !matches!(op, b'M' | b'm') {
            out.push(Command::MoveTo(start));
        }
        closed = false;
        // One command letter may be followed by several argument sets.
        loop {
            let origin = if op.is_ascii_lowercase() { cur } else { zero };
            let (mut next_cubic, mut next_quad) = (None, None);
            match op.to_ascii_uppercase() {
                b'M' => {
                    cur = s.point(origin)?;
                    start = cur;
                    out.push(Command::MoveTo(cur));
                    // Further pairs are line-tos.
                    op = if op == b'm' { b'l' } else { b'L' };
                }
                b'L' => {
                    cur = s.point(origin)?;
                    out.push(Command::LineTo(cur));
                }
                b'H' => {
                    cur = Point::new(origin.x + s.number()?, cur.y);
                    out.push(Command::LineTo(cur));
                }
                b'V' => {
                    cur = Point::new(cur.x, origin.y + s.number()?);
                    out.push(Command::LineTo(cur));
                }
                b'C' | b'S' => {
                    let c1 = if op.eq_ignore_ascii_case(&b'C') {
                        s.point(origin)?
                    } else {
                        reflect(cur, cubic)
                    };
                    let c2 = s.point(origin)?;
                    cur = s.point(origin)?;
                    out.push(Command::CurveTo(c1, c2, cur));
                    next_cubic = Some(c2);
                }
                b'Q' | b'T' => {
                    let c = if op.eq_ignore_ascii_case(&b'Q') {
                        s.point(origin)?
                    } else {
                        reflect(cur, quad)
                    };
                    cur = s.point(origin)?;
                    out.push(Command::QuadTo(c, cur));
                    next_quad = Some(c);
                }
                _ => {
                    let rx = s.number()?;
                    let ry = s.number()?;
                    let angle = Angle::from_degrees(s.number()?);
                    let size = if s.flag()? {
                        ArcSize::Large
                    } else {
                        ArcSize::Small
                    };
                    let sweep = if s.flag()? {
                        ArcSweep::Positive
                    } else {
                        ArcSweep::Negative
                    };
                    cur = s.point(origin)?;
                    out.arc_to(rx.abs(), ry.abs(), angle, size, sweep, cur);
                }
            }
            (cubic, quad) = (next_cubic, next_quad);
            s.skip_separators();
            if !s
                .peek()
                .is_some_and(|c| c.is_ascii_digit() || b"+-.".contains(&c))
            {
                break;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn repeats_relative_coordinates_and_close() {
        let got = parse_path("m1 1 2 0v2H1zl1 1").unwrap();
        assert_eq!(
            got,
            vec![
                Command::MoveTo(p(1.0, 1.0)),
                Command::LineTo(p(3.0, 1.0)),
                Command::LineTo(p(3.0, 3.0)),
                Command::LineTo(p(1.0, 3.0)),
                Command::Close,
                Command::MoveTo(p(1.0, 1.0)),
                Command::LineTo(p(2.0, 2.0)),
            ]
        );
    }

    #[test]
    fn smooth_curves_reflect_only_after_a_curve() {
        // A repeated `s` reflects the previous control point.
        let got = parse_path("M0 0s1 1 2 0 3-1 4 0").unwrap();
        assert_eq!(
            got[1],
            Command::CurveTo(p(0.0, 0.0), p(1.0, 1.0), p(2.0, 0.0))
        );
        assert_eq!(
            got[2],
            Command::CurveTo(p(3.0, -1.0), p(5.0, -1.0), p(6.0, 0.0))
        );
        // After a line the first control point is the current point.
        let got = parse_path("M0 0C0 1 1 1 1 0L2 0S3 1 4 0").unwrap();
        assert_eq!(
            got[3],
            Command::CurveTo(p(2.0, 0.0), p(3.0, 1.0), p(4.0, 0.0))
        );
    }

    #[test]
    fn compact_numbers_and_arc_flags() {
        assert_eq!(
            parse_path("M.5.5L1-1").unwrap()[1],
            Command::LineTo(p(1.0, -1.0))
        );
        assert_eq!(
            parse_path("M0 0a1 1 0 012 0").unwrap(),
            parse_path("M0 0a1,1,0,0,1,2,0").unwrap()
        );
    }

    #[test]
    fn malformed_data_is_an_error() {
        for d in [
            "L1 1",
            "M1",
            "M0 0L1",
            "M0 0C1 1 2 2",
            "M0 0X1 1",
            "M0 0a1 1 0 2 0 1 1",
            "M0 0z1 1",
            "M0 0L1 1e",
        ] {
            assert!(parse_path(d).is_err(), "{d} was accepted");
        }
    }
}
