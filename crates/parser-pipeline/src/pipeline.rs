use crate::collector::{Collector, Limits, PageData};
use crate::detector::challenge_match;
use crate::dom::{self, AttrNames, TagNames};
use crate::scratch::{self, Ev};
use lol_html::MemorySettings;
use lol_html::html_content::TextChunk;
use lol_html::send::Element;

const REWRITE_MEM_FLOOR: u64 = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Stop,
}

#[derive(Debug, thiserror::Error)]
pub enum PipeError {
    #[error("memory brake")]
    MemoryBrake,
    #[error("rewriter misuse")]
    Misuse,
}

struct NullSink;

impl lol_html::OutputSink for NullSink {
    #[inline(always)]
    fn handle_chunk(&mut self, _chunk: &[u8]) {}
}

type Rewriter = lol_html::send::HtmlRewriter<'static, NullSink>;

struct Utf8StreamState {
    tail: [u8; 4],
    len: u8,
}

impl Utf8StreamState {
    #[inline]
    fn new() -> Self {
        Self {
            tail: [0; 4],
            len: 0,
        }
    }

    fn check(&mut self, mut chunk: &[u8]) -> bool {
        let mut ok = true;
        while !chunk.is_empty() {
            if self.len == 0 {
                match core_utils::utf8::compat::from_utf8(chunk) {
                    Ok(_) => return ok,
                    Err(e) => {
                        if e.error_len().is_none() {
                            let start = e.valid_up_to();
                            let rem = chunk.len() - start;
                            if rem <= 4 {
                                self.tail[..rem].copy_from_slice(&chunk[start..]);
                                self.len = rem as u8;
                                return ok;
                            }
                            return false;
                        }

                        ok = false;
                        let skip = e.valid_up_to() + e.error_len().unwrap_or(1);
                        chunk = &chunk[skip..];
                    }
                }
            } else {
                let tl = self.len as usize;
                let take = chunk.len().min(8 - tl);
                let mut joined = [0u8; 8];
                joined[..tl].copy_from_slice(&self.tail[..tl]);
                joined[tl..tl + take].copy_from_slice(&chunk[..take]);
                let j = tl + take;
                match core_utils::utf8::compat::from_utf8(&joined[..j]) {
                    Ok(_) => {
                        self.len = 0;
                        chunk = &chunk[take..];
                    }
                    Err(e) => {
                        if e.error_len().is_none() {
                            let start = e.valid_up_to();
                            let rem = j - start;
                            if rem <= 4 && start + rem == j {
                                self.tail[..rem].copy_from_slice(&joined[start..j]);
                                self.len = rem as u8;
                                chunk = &chunk[take..];
                            } else {
                                return false;
                            }
                        } else {
                            ok = false;
                            let skip = e.valid_up_to() + e.error_len().unwrap_or(1);
                            if skip < tl {
                                self.len = 0;
                            } else {
                                let cs = (skip - tl).max(1);
                                self.len = 0;
                                chunk = &chunk[cs..];
                            }
                        }
                    }
                }
            }
        }
        ok
    }
}

pub struct StreamPipeline {
    rewriter: Option<Rewriter>,
    collector: Collector,
    limits: Limits,
    bytes_fed: u64,
    utf8_bad_chunks: u32,
    utf8_state: Utf8StreamState,
    braked: bool,
}

impl StreamPipeline {
    pub fn new(limits: Limits) -> Self {
        Self::build(limits, &[])
    }

    pub fn with_selectors(limits: Limits, selectors: &[(String, String)]) -> Self {
        Self::build(limits, selectors)
    }

    fn build(limits: Limits, selectors: &[(String, String)]) -> Self {
        let extract_keys: Vec<compact_str::CompactString> = selectors
            .iter()
            .map(|(k, _)| compact_str::CompactString::new(k.as_str()))
            .collect();

        let mut settings = lol_html::send::Settings::new_send()
            .with_memory_settings(
                MemorySettings::new()
                    .with_max_allowed_memory_usage(limits.byte_brake.max(REWRITE_MEM_FLOOR) as usize)
                    .with_graceful_bail_out_on_memory_limit_exceeded(true),
            )
            .with_graceful_bail_out_on_content_handler_error(true)
            .append_element_content_handler(lol_html::element!("*", move |el: &mut Element<
                '_,
                '_,
            >| {
                let (tag, tag_dyn) = dom::push_name::<TagNames>(el.tag_name().as_str());
                let attr_start = scratch::attr_mark();
                let mut count: u8 = 0;
                for a in el.attributes() {
                    let (aid, adyn) = dom::push_name::<AttrNames>(a.name().as_str());
                    scratch::push_attr(scratch::AttrEv {
                        name: aid,
                        name_dyn: adyn,
                        value: scratch::push_str(&a.value()),
                    });
                    count += 1;
                    if count == u8::MAX {
                        break;
                    }
                }
                scratch::emit(Ev::DomOpen {
                    tag,
                    tag_dyn,
                    attr_start,
                    attr_count: count,
                });
                if !dom::is_void_tag(tag) {
                    let _ = el.on_end_tag(lol_html::end_tag!(move |end| {
                        let (t, tdyn) = dom::push_name::<TagNames>(end.name().as_str());
                        scratch::emit(Ev::DomClose {
                            tag: t,
                            tag_dyn: tdyn,
                        });
                        Ok(())
                    }));
                }
                Ok(())
            }))
            .append_element_content_handler(lol_html::element!(
                "script",
                move |el: &mut Element<'_, '_>| {
                    let kind = match el.get_attribute("id").as_deref() {
                        Some("__NEXT_DATA__") => scratch::ScriptEvKind::NextData,
                        Some("anubis_challenge") => scratch::ScriptEvKind::Anubis,
                        Some("anubis_version") => scratch::ScriptEvKind::AnubisVersion,
                        _ => scratch::ScriptEvKind::Inline,
                    };
                    let src_raw = el.get_attribute("src");
                    if let Some(raw) = &src_raw
                        && challenge_match(raw)
                    {
                        let marker = scratch::push_str(raw);
                        scratch::emit(Ev::ChallengeDetected { marker });
                    }
                    let src = src_raw.map(|s| scratch::push_str(&s));
                    scratch::emit(Ev::Script { src, kind });
                    Ok(())
                }
            ))
            .append_element_content_handler(lol_html::element!("form", move |el: &mut Element<
                '_,
                '_,
            >| {
                let action = el.get_attribute("action").map(|a| scratch::push_str(&a));
                let method = el
                    .get_attribute("method")
                    .map(|mut m| {
                        m.make_ascii_lowercase();
                        scratch::push_str(&m)
                    })
                    .unwrap_or(scratch::NIL_SPAN);
                scratch::emit(Ev::Form { action, method });
                Ok(())
            }))
            .append_element_content_handler(lol_html::element!(
                "input, select, textarea, button",
                move |el: &mut Element<'_, '_>| {
                    let tag = el.tag_name();
                    let name = el.get_attribute("name").unwrap_or_default();
                    if name.is_empty() {
                        return Ok(());
                    }
                    let kind = el.get_attribute("type").unwrap_or_default();
                    let hidden = tag == "input" && dom::is_hidden_type(kind.as_bytes());
                    scratch::emit(Ev::Field {
                        name: scratch::push_str(&name),
                        value: el.get_attribute("value").map(|v| scratch::push_str(&v)),
                        kind: scratch::push_str(&kind),
                        hidden,
                    });
                    Ok(())
                }
            ))
            .append_element_content_handler(lol_html::element!(
                "title",
                move |_el: &mut Element<'_, '_>| {
                    scratch::emit(Ev::TitleOpen);
                    Ok(())
                }
            ))
            .append_element_content_handler(lol_html::element!(
                "meta[name='description']",
                move |el: &mut Element<'_, '_>| {
                    if let Some(d) = el.get_attribute("content") {
                        scratch::emit(Ev::MetaDescription {
                            span: scratch::push_str(&d),
                        });
                    }
                    Ok(())
                }
            ))
            .append_element_content_handler(lol_html::text!("script", move |t: &mut TextChunk<
                '_,
            >| {
                let last = t.last_in_text_node();
                let span = scratch::push_str(t.as_str());
                scratch::emit(Ev::ScriptText { span, last });
                Ok(())
            }))
            .append_element_content_handler(lol_html::text!("title", move |t: &mut TextChunk<
                '_,
            >| {
                let last = t.last_in_text_node();
                let span = scratch::push_str(t.as_str());
                scratch::emit(Ev::TitleText { span, last });
                Ok(())
            }))
            .append_element_content_handler(lol_html::text!("*", move |t: &mut TextChunk<'_>| {
                let span = scratch::push_str(t.as_str());
                scratch::emit(Ev::DomText { span });
                Ok(())
            }));

        for (i, (_, sel)) in selectors.iter().enumerate() {
            let key = i as u32;
            let sel = sel.clone();
            settings = settings.append_element_content_handler(lol_html::text!(
                sel,
                move |t: &mut TextChunk<'_>| {
                    let span = scratch::push_str(t.as_str());
                    scratch::emit(Ev::Extract { key, span });
                    Ok(())
                }
            ));
        }

        let rewriter = lol_html::send::HtmlRewriter::new(settings, NullSink);

        Self {
            rewriter: Some(rewriter),
            collector: Collector::new(extract_keys),
            limits,
            bytes_fed: 0,
            utf8_bad_chunks: 0,
            utf8_state: Utf8StreamState::new(),
            braked: false,
        }
    }

    #[inline(always)]
    fn drain(&mut self) {
        scratch::drain_into(&mut self.collector);
    }

    #[inline(always)]
    pub fn push(&mut self, chunk: &[u8]) -> Result<Flow, PipeError> {
        let Some(rewriter) = self.rewriter.as_mut() else {
            if self.braked {
                return Ok(Flow::Stop);
            }
            return Err(PipeError::Misuse);
        };
        if self.bytes_fed >= self.limits.byte_brake {
            return Ok(Flow::Stop);
        }
        let remaining = self.limits.byte_brake - self.bytes_fed;
        let take = remaining.min(chunk.len() as u64) as usize;
        let accepted = &chunk[..take];
        self.bytes_fed += take as u64;
        if !self.utf8_state.check(accepted) {
            self.utf8_bad_chunks = self.utf8_bad_chunks.saturating_add(1);
        }
        let outcome = rewriter.write(accepted);
        self.drain();
        if outcome.is_err() {
            self.rewriter = None;
            self.braked = true;
            return Ok(Flow::Stop);
        }
        if self.bytes_fed >= self.limits.byte_brake {
            return Ok(Flow::Stop);
        }
        Ok(Flow::Continue)
    }

    pub fn finish(mut self) -> Result<PageData, PipeError> {
        if let Some(rewriter) = self.rewriter.take() {
            let outcome = rewriter.end();
            self.drain();
            if outcome.is_err() {
                self.braked = true;
            }
        } else if !self.braked {
            return Err(PipeError::Misuse);
        }
        let truncated = self.braked || self.bytes_fed >= self.limits.byte_brake;
        Ok(self
            .collector
            .into_page(self.bytes_fed, self.utf8_bad_chunks, truncated))
    }
}
