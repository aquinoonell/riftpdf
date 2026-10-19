use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use mupdf::{Colorspace, Document, Matrix, Outline};

#[derive(Clone)]
pub struct PageBitmap {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u32>>, // 0RGB softbuffer layout
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub page: usize,
    /// Axis-aligned box in page user space (pre-scale)
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[derive(Clone, Debug)]
pub struct TocEntry {
    pub title: String,
    pub page: usize,
    pub depth: usize,
}

#[derive(Clone, Debug)]
pub struct DocLink {
    pub page: usize,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub uri: String,
    /// Internal jump target, if any.
    pub dest_page: Option<usize>,
}

pub struct PdfDoc {
    doc: Document,
    page_count: usize,
    cache: HashMap<CacheKey, PageBitmap>,
    cache_order: Vec<CacheKey>,
    cache_cap: usize,
    toc: Vec<TocEntry>,
    link_cache: HashMap<usize, Vec<DocLink>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    page: usize,
    scale_milli: u32,
}

impl PdfDoc {
    pub fn open(path: &Path) -> Result<Self, mupdf::Error> {
        let doc = Document::open(path)?;
        let page_count = doc.page_count()? as usize;
        let toc = flatten_toc(&doc.outlines().unwrap_or_default(), 0);
        Ok(Self {
            doc,
            page_count: page_count.max(1),
            cache: HashMap::new(),
            cache_order: Vec::new(),
            cache_cap: 5,
            toc,
            link_cache: HashMap::new(),
        })
    }

    pub fn page_count(&self) -> usize {
        self.page_count
    }

    pub fn toc(&self) -> &[TocEntry] {
        &self.toc
    }

    pub fn page_size(&self, page: usize) -> Result<(f32, f32), mupdf::Error> {
        let page = self.doc.load_page(page as i32)?;
        let bounds = page.bounds()?;
        Ok((bounds.width(), bounds.height()))
    }

    pub fn links_on_page(&mut self, page: usize) -> Result<&[DocLink], mupdf::Error> {
        if !self.link_cache.contains_key(&page) {
            let loaded = self.doc.load_page(page as i32)?;
            let mut links = Vec::new();
            for link in loaded.links()? {
                let b = link.bounds;
                links.push(DocLink {
                    page,
                    x0: b.x0,
                    y0: b.y0,
                    x1: b.x1,
                    y1: b.y1,
                    uri: link.uri,
                    dest_page: link.dest.map(|d| d.loc.page_number as usize),
                });
            }
            self.link_cache.insert(page, links);
        }
        Ok(self
            .link_cache
            .get(&page)
            .map(|v| v.as_slice())
            .unwrap_or(&[]))
    }

    pub fn search(&self, needle: &str) -> Result<Vec<SearchHit>, mupdf::Error> {
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        for page_idx in 0..self.page_count {
            let page = self.doc.load_page(page_idx as i32)?;
            let quads = page.search(needle, 256)?;
            for q in quads.iter() {
                let xs = [q.ul.x, q.ur.x, q.ll.x, q.lr.x];
                let ys = [q.ul.y, q.ur.y, q.ll.y, q.lr.y];
                hits.push(SearchHit {
                    page: page_idx,
                    x0: xs.iter().copied().fold(f32::INFINITY, f32::min),
                    y0: ys.iter().copied().fold(f32::INFINITY, f32::min),
                    x1: xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
                    y1: ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
                });
            }
        }
        Ok(hits)
    }

    pub fn render(&mut self, page: usize, scale: f32) -> Result<PageBitmap, mupdf::Error> {
        let scale = scale.clamp(0.1, 8.0);
        let key = CacheKey {
            page,
            scale_milli: (scale * 1000.0).round() as u32,
        };

        if let Some(hit) = self.cache.get(&key) {
            if let Some(i) = self.cache_order.iter().position(|k| *k == key) {
                let k = self.cache_order.remove(i);
                self.cache_order.push(k);
            }
            return Ok(hit.clone());
        }

        let bmp = self.render_uncached(page, scale)?;
        self.insert_cache(key, bmp.clone());
        let _ = self.prefetch(page.wrapping_sub(1), scale);
        let _ = self.prefetch(page + 1, scale);
        Ok(bmp)
    }

    fn prefetch(&mut self, page: usize, scale: f32) -> Result<(), mupdf::Error> {
        if page >= self.page_count {
            return Ok(());
        }
        let key = CacheKey {
            page,
            scale_milli: (scale * 1000.0).round() as u32,
        };
        if self.cache.contains_key(&key) {
            return Ok(());
        }
        let bmp = self.render_uncached(page, scale)?;
        self.insert_cache(key, bmp);
        Ok(())
    }

    fn render_uncached(&self, page: usize, scale: f32) -> Result<PageBitmap, mupdf::Error> {
        let page = self.doc.load_page(page as i32)?;
        let pixmap = page.to_pixmap(
            &Matrix::new_scale(scale, scale),
            &Colorspace::device_rgb(),
            false,
            true,
        )?;
        let w = pixmap.width();
        let h = pixmap.height();
        let samples = pixmap.samples();
        let n = pixmap.n() as usize;

        let mut pixels = Vec::with_capacity((w * h) as usize);
        for chunk in samples.chunks(n) {
            let r = chunk[0] as u32;
            let g = chunk.get(1).copied().unwrap_or(0) as u32;
            let b = chunk.get(2).copied().unwrap_or(0) as u32;
            pixels.push((r << 16) | (g << 8) | b);
        }

        Ok(PageBitmap {
            width: w,
            height: h,
            pixels: Arc::new(pixels),
        })
    }

    fn insert_cache(&mut self, key: CacheKey, bmp: PageBitmap) {
        if !self.cache.contains_key(&key) {
            self.cache_order.push(key);
        }
        self.cache.insert(key, bmp);
        while self.cache_order.len() > self.cache_cap {
            if let Some(old) = self.cache_order.first().copied() {
                self.cache_order.remove(0);
                self.cache.remove(&old);
            } else {
                break;
            }
        }
    }
}

fn flatten_toc(outlines: &[Outline], depth: usize) -> Vec<TocEntry> {
    let mut out = Vec::new();
    for o in outlines {
        let page = o
            .dest
            .as_ref()
            .map(|d| d.loc.page_number as usize)
            .unwrap_or(0);
        out.push(TocEntry {
            title: o.title.clone(),
            page,
            depth,
        });
        out.extend(flatten_toc(&o.down, depth + 1));
    }
    out
}
