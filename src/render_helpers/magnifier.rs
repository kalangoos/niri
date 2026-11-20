//! KalangoOS magnifier (design "Lupa", Super and +): a framebuffer effect that copies the part of the screen around
//! the pointer — what is already drawn behind it — scaled up into its own rectangle. Everything in front of it (the
//! pointer, the lens border) is drawn after, at its normal size.

use std::cell::RefCell;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element, Id, RenderElement};
use smithay::backend::renderer::gles::{ffi, GlesError, GlesFrame, GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{Frame as _, FrameContext, Offscreen, Texture as _};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::renderer::AsGlesFrame as _;

#[derive(Debug)]
pub struct MagnifierElement {
    id: Id,
    commit: CommitCounter,
    /// where the lens is drawn (output logical coordinates)
    geometry: Rectangle<f64, Logical>,
    /// the part of the screen it shows, scaled up to `geometry` (output logical coordinates)
    source: Rectangle<f64, Logical>,
    scale: f64,
}

#[derive(Debug, Default)]
struct Inner {
    texture: Option<GlesTexture>,
}

/// The part of an output of `size` shown by a lens of `lens` size at `factor`, centred on `pointer` and kept inside
/// the output.
pub fn source_rect(
    pointer: Point<f64, Logical>,
    lens: Size<f64, Logical>,
    factor: f64,
    size: Size<f64, Logical>,
) -> Rectangle<f64, Logical> {
    let w = (lens.w / factor).min(size.w);
    let h = (lens.h / factor).min(size.h);
    let x = (pointer.x - w / 2.).clamp(0., (size.w - w).max(0.));
    let y = (pointer.y - h / 2.).clamp(0., (size.h - h).max(0.));
    Rectangle::new(Point::from((x, y)), Size::from((w, h)))
}

impl MagnifierElement {
    pub fn new(
        id: Id,
        commit: CommitCounter,
        geometry: Rectangle<f64, Logical>,
        source: Rectangle<f64, Logical>,
        scale: f64,
    ) -> Self {
        Self {
            id,
            commit,
            geometry,
            source,
            scale,
        }
    }
}

impl Element for MagnifierElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(self.geometry.size.to_buffer(1., Transform::Normal))
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(scale)
    }

    fn is_framebuffer_effect(&self) -> bool {
        true
    }
}

impl RenderElement<GlesRenderer> for MagnifierElement {
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        let output_rect = Rectangle::from_size(frame.output_size());
        let transform = frame.transformation();
        let source: Rectangle<i32, Physical> =
            self.source.to_physical_precise_round(Scale::from(self.scale));
        let Some(source) = source.intersection(output_rect) else {
            return Ok(());
        };
        let source = transform.transform_rect_in(source, &output_rect.size);
        let size: Size<i32, Buffer> = transform
            .transform_size(dst.size)
            .to_logical(1)
            .to_buffer(1, Transform::Normal);

        let mut guard = frame.renderer();
        let inner = cache.get_or_insert::<RefCell<Inner>, _>(|| RefCell::new(Inner::default()));
        let mut inner = inner.borrow_mut();
        if inner.texture.as_ref().is_some_and(|t| t.size() != size) {
            inner.texture = None;
        }
        if inner.texture.is_none() {
            let t = guard.as_mut().create_buffer(Fourcc::Abgr8888, size)?;
            inner.texture = Some(t);
        }
        let tex_id = inner.texture.as_ref().unwrap().tex_id();
        drop(guard);

        // Copy the part of the screen around the pointer into the texture, scaled up (linear filtering).
        frame.with_context(|gl| unsafe {
            while gl.GetError() != ffi::NO_ERROR {}
            let mut current_fbo = 0i32;
            gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut current_fbo as *mut _);
            gl.Disable(ffi::SCISSOR_TEST);
            let mut fbo = 0;
            gl.GenFramebuffers(1, &mut fbo as *mut _);
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbo);
            gl.FramebufferTexture2D(
                ffi::DRAW_FRAMEBUFFER,
                ffi::COLOR_ATTACHMENT0,
                ffi::TEXTURE_2D,
                tex_id,
                0,
            );
            gl.BlitFramebuffer(
                source.loc.x,
                source.loc.y,
                source.loc.x + source.size.w,
                source.loc.y + source.size.h,
                0,
                0,
                size.w,
                size.h,
                ffi::COLOR_BUFFER_BIT,
                ffi::LINEAR,
            );
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, current_fbo as u32);
            gl.Enable(ffi::SCISSOR_TEST);
            gl.DeleteFramebuffers(1, &mut fbo as *mut _);
            if gl.GetError() != ffi::NO_ERROR {
                Err(GlesError::BlitError)
            } else {
                Ok(())
            }
        })??;
        Ok(())
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let Some(inner) = cache.and_then(|c| c.get::<RefCell<Inner>>()) else {
            return Ok(());
        };
        let inner = inner.borrow();
        let Some(texture) = &inner.texture else {
            return Ok(());
        };
        frame.render_texture_from_to(
            texture,
            Rectangle::from_size(texture.size().to_f64()),
            dst,
            damage,
            &[],
            // the texture is in the frame's orientation
            frame.transformation().invert(),
            1.,
            None,
            &[],
        )
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for MagnifierElement {
    fn capture_framebuffer(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), TtyRendererError<'render>> {
        let gles_frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::capture_framebuffer(self, gles_frame, src, dst, cache)?;
        Ok(())
    }

    fn draw(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), TtyRendererError<'render>> {
        let gles_frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::draw(
            self,
            gles_frame,
            src,
            dst,
            damage,
            opaque_regions,
            cache,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lens_shows_the_part_around_the_pointer_inside_the_screen() {
        let screen = Size::from((1920., 1080.));
        let lens = Size::from((620., 380.));
        let r = source_rect(Point::from((870., 490.)), lens, 2.5, screen);
        assert_eq!((r.size.w, r.size.h), (248., 152.));
        assert_eq!((r.loc.x, r.loc.y), (746., 414.));
        // at a corner it stays inside
        let r = source_rect(Point::from((5., 5.)), lens, 2.5, screen);
        assert_eq!((r.loc.x, r.loc.y), (0., 0.));
    }
}
