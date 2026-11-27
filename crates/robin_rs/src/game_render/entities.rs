//! entities presentation pass.
use super::*;

fn npc_in_building(engine: &PresentationView<'_>, entity: &Entity) -> bool {
    entity.is_npc()
        && entity.element_data().sector().is_some_and(|sector| {
            engine
                .fast_grid()
                .sector_type_for_handle(sector)
                .is_building()
        })
}

pub(super) fn render_character_masks_clipped(
    engine: &PresentationView<'_>,
    renderer: &mut Renderer,
    layer: u16,
    world_bbox: &engine_coordinates::MapBBox,
    position: engine_coordinates::MapPoint,
    clip_rect: Rect,
    draw_checkpoint: usize,
    view: engine_coordinates::MapPoint,
    zoom: f32,
    rotation: CameraRotation,
) {
    let mask_indices = engine
        .fast_grid()
        .get_masks_applied_to_character(layer, world_bbox, position);
    if mask_indices.is_empty() {
        return;
    }
    let screen_masks = sprite_screen_masks(engine, &mask_indices, view, zoom, rotation);
    renderer.mask_queued_draws(draw_checkpoint, &screen_masks, clip_rect);
}

pub(super) fn sprite_screen_masks(
    engine: &PresentationView<'_>,
    mask_indices: &[engine_mask::MaskIndex],
    view: engine_coordinates::MapPoint,
    zoom: f32,
    rotation: CameraRotation,
) -> Vec<(u32, Rect)> {
    let mut screen_masks = Vec::with_capacity(mask_indices.len());
    for &mask_idx in mask_indices {
        let mask = &engine.fast_grid().level.masks[usize::from(mask_idx)];
        let bounds = map_sprite_bounds(
            MapPoint::new(mask.bbox.x_min(), mask.bbox.y_min()),
            mask.width as u32,
            mask.height as u32,
            view,
            zoom,
            rotation,
        );
        let mask_screen_x = bounds.min.x.round() as i32;
        let mask_screen_y = bounds.min.y.round() as i32;
        let mask_screen_w = (bounds.max.x - bounds.min.x).round().max(0.0) as u32;
        let mask_screen_h = (bounds.max.y - bounds.min.y).round().max(0.0) as u32;
        if mask_screen_w == 0 || mask_screen_h == 0 {
            continue;
        }
        let mask_rect = Rect::new(mask_screen_x, mask_screen_y, mask_screen_w, mask_screen_h);
        screen_masks.push((u32::from(mask_idx), mask_rect));
    }
    screen_masks
}

pub(super) fn applicable_sprite_masks(
    engine: &PresentationView<'_>,
    assets: &LevelAssets,
    actor_layer: Option<u16>,
    sprite_world_bbox: &engine_coordinates::MapBBox,
    actor_position: engine_coordinates::MapPoint,
    projectile_position: engine_coordinates::WorldPoint3D,
    use_projectile_path: bool,
    is_flying_human: bool,
) -> Vec<engine_mask::MaskIndex> {
    if use_projectile_path {
        engine.fast_grid().get_masks_applied_to_projectile(
            engine.fast_grid().level.special_layer,
            sprite_world_bbox,
            projectile_position,
            is_flying_human,
            engine.sight_obstacles(assets),
        )
    } else {
        let Some(actor_layer) = actor_layer else {
            // A layerless entity cannot use the character mask query. This
            // is valid while an entity is detached from world membership;
            // leave its sprite unmasked until it acquires a layer.
            return Vec::new();
        };
        engine.fast_grid().get_masks_applied_to_character(
            actor_layer,
            sprite_world_bbox,
            actor_position,
        )
    }
}

// ─── GPU entity rendering ─────────────────────────────────────────

pub(super) fn entity_visual_map_position(entity: &Entity) -> MapPoint {
    entity.sprite_visual_map_position()
}

/// Render all entities using cached GPU textures.
///
/// Replaces `render_entities` for the GPU phase.  Each sprite frame is
/// decompressed once and cached as an ARGB8888 GPU texture; subsequent
/// frames with the same `(bank_id, variant, shadow_color)` key reuse the
/// cached texture in a queued GPU draw (zero CPU decompression work).
pub(crate) fn render_entities_gpu(
    host: &HostDraw<'_>,
    presentation: &FramePresentationInputs,
    engine: &PresentationView<'_>,
    assets: &LevelAssets,
    dev: &DevState,
    renderer: &mut Renderer,
    titbit_renderer: &mut TitbitRenderer,
) {
    let ctx = EntityPassCtx {
        host,
        engine,
        assets,
        dev,
        frame: EntityPassFrame {
            view: presentation.view,
            zoom: presentation.zoom,
            rotation: presentation.rotation,
            screen_w: presentation.screen_size.x as i32,
            screen_h: presentation.screen_size.y as i32,
            shadow_color: presentation.shadow_color,
            global_shadow: host.frontend.resources.frame_holder().global_shadow(),
            blip_shadow: host.frontend.resources.frame_holder().global_blip_shadow(),
        },
    };
    let EntityPassFrame {
        view,
        zoom,
        rotation,
        screen_w,
        screen_h,
        shadow_color,
        ..
    } = ctx.frame;
    // When the player has disabled "Display Animations" in the graphics
    // options, unforced non-patched non-elevated non-masked FX should
    // not render.  The flag defaults to `true` so the live datadir is
    // unaffected; it only bites when the user toggles it off in the
    // options menu.
    let graphic_config = &presentation.graphic_config;
    let display_anim = graphic_config.display_anim;
    let apply_fog_to_all_sprites = graphic_config.apply_fog_to_all_sprites;

    // Ground-only overlay: drawn before sprites so characters stand on top
    // of the dotted boundary, and recomputed from this frame's inputs only.
    render_hidden_ground_outlines(host, engine, assets, dev, renderer);

    for &entity_id in &presentation.draw_order_ids {
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        if !entity.is_active() || entity.element_data().hidden_in_building {
            continue;
        }
        if npc_in_building(engine, entity) {
            continue;
        }
        if !engine.fog_entity_visible(entity_id) && !uses_pixel_fog_visibility(entity) {
            continue;
        }
        // FX entities early-return when `is_to_be_displayed` is false;
        // non-FX kinds always pass.
        if !entity.is_to_be_displayed(display_anim) {
            continue;
        }
        let variant = engine.resolve_render_variant_for_ambiance(
            entity,
            apply_fog_to_all_sprites,
            presentation.ambiance,
        );

        // ── Interleave titbits that belong behind this entity ─────
        // Immediately before drawing each human entity, flush any
        // pending titbits whose depth falls behind this entity's so
        // they render back-to-front with the entity list (projectile /
        // dust / stars sit between actors at the correct depth instead
        // of piled on top at the end).
        if entity.is_human() {
            titbit_renderer.render_up_to(
                host,
                engine,
                assets,
                renderer,
                entity.sprite().display_depth,
            );
        }

        let elem = entity.element_data();
        let visual_pos = entity_visual_map_position(entity);
        let world_x = visual_pos.x;
        let world_y = visual_pos.y;
        if world_x == 0.0 && world_y == 0.0 {
            continue;
        }

        let screen = rotation.to_screen(MapPoint::new(world_x, world_y), view, zoom);
        let screen_x = screen.x as i32;
        let screen_y = screen.y as i32;

        if outside_sprite_cull_margin((screen_x, screen_y), (screen_w, screen_h), zoom) {
            continue;
        }

        // Try GPU sprite rendering.
        //
        // Using `current_scripts_opt` (not a direct `.scripts` field
        // read) is essential for blipped NPCs: `load_frame_info` stores
        // the normal character as primary + `blip00` as alternate, and
        // flips `use_alternate_profile` so the blip silhouette is the
        // active profile until reveal flips it back.  A direct
        // field read would show the revealed character even while it
        // should still be a shadow.  Always go through the active-
        // profile pointer.
        let sprite = &elem.sprite;
        let Some((script, frame, bank_id)) =
            current_render_frame(sprite, ctx.host.viewport().camera_steps())
        else {
            render_entity_fallback(
                renderer,
                entity.kind(),
                screen_x,
                screen_y,
                screen_w,
                screen_h,
            );
            continue;
        };

        let shadow_level = entity_shadow_level(entity, &ctx.frame);
        let Some(size) = renderer.ensure_sprite_cached(
            host.frontend.resources.frame_holder(),
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        ) else {
            render_entity_fallback(
                renderer,
                entity.kind(),
                screen_x,
                screen_y,
                screen_w,
                screen_h,
            );
            continue;
        };
        render_cached_entity_sprite(
            &ctx,
            renderer,
            &CachedEntitySprite {
                entity,
                script,
                frame,
                bank_id,
                variant,
                shadow_level,
                size,
                world: MapPoint::new(world_x, world_y),
            },
        );
    }
}

/// Frame-wide inputs of the GPU entity pass, read once before the draw loop.
#[derive(Clone, Copy)]
struct EntityPassFrame {
    view: MapPoint,
    zoom: f32,
    rotation: CameraRotation,
    screen_w: i32,
    screen_h: i32,
    shadow_color: u16,
    global_shadow: u16,
    blip_shadow: u16,
}

/// Borrowed world/host state shared by the per-entity phases of the pass.
struct EntityPassCtx<'a, 'h, 'e> {
    host: &'a HostDraw<'h>,
    engine: &'a PresentationView<'e>,
    assets: &'a LevelAssets,
    dev: &'a DevState,
    frame: EntityPassFrame,
}

/// An entity whose current sprite frame is cached on the GPU.
struct CachedEntitySprite<'a> {
    entity: &'a Entity,
    script: &'a robin_engine::sprite_script::SpriteScript,
    frame: u16,
    bank_id: u32,
    variant: robin_engine::sprite_variant::SpriteVariant,
    shadow_level: u16,
    size: (u16, u16),
    /// Visual map anchor from [`entity_visual_map_position`].
    world: MapPoint,
}

/// Mask-relevant facts shared by the teleport ghost and the current sprite.
#[derive(Clone, Copy)]
struct EntityMaskFacts {
    actor_layer: Option<u16>,
    is_flying_human: bool,
    hidden_outline_rgb: Option<(u8, u8, u8)>,
}

/// Blipped (undiscovered) NPCs render from the `blip00`
/// alternate profile as a silhouette sprite; the alpha-keying
/// pass uses the global blip shadow (60) for this branch vs the
/// global shadow (40) for normal characters.
///
/// FX entities switch on `rendering_properties`: `NeedShadow`
/// composites a shadow, `Blocky` doesn't.  Zero `shadow_level`
/// for `Blocky` FX so the cached sprite key drops the shadow
/// tint.
fn entity_shadow_level(entity: &Entity, frame: &EntityPassFrame) -> u16 {
    let mut shadow_level = if entity.element_data().sprite.use_alternate_profile {
        frame.blip_shadow
    } else {
        frame.global_shadow
    };
    if matches!(entity.kind(), ElementKind::Fx)
        && let Some(fx) = entity.fx_data()
        && fx.rendering_properties == RenderingProperties::Blocky
    {
        shadow_level = 0;
    }
    shadow_level
}

/// Draw one GPU-cached entity sprite: the teleport ghost (if any), the
/// sprite itself, then the building-occlusion mask pass.
fn render_cached_entity_sprite(
    ctx: &EntityPassCtx<'_, '_, '_>,
    renderer: &mut Renderer,
    cached: &CachedEntitySprite<'_>,
) {
    let EntityPassFrame {
        view,
        zoom,
        rotation,
        shadow_color,
        ..
    } = ctx.frame;
    let entity = cached.entity;
    let elem = entity.element_data();
    let (sw, sh) = cached.size;
    // Sprite screen position:
    //   sprite_pos  = floor(position_map - sprite.center)
    //   blit_origin = sprite_pos + script_offset
    //   screen_xy   = (blit_origin - view) * zoom
    // The floor() in world space (before zoom) is critical for
    // pixel-perfect alignment.
    let placement = sprite_placement(
        cached.world,
        elem.sprite.center,
        cached.script.offsets[cached.frame as usize],
        view,
        zoom,
        rotation,
    );
    let (dst_x, dst_y) = placement.screen_origin;

    let dst_rect = zoomed_sprite_rect(dst_x, dst_y, sw, sh, zoom);
    let kind = entity.kind();
    let facts = EntityMaskFacts {
        actor_layer: elem.optional_layer().map(Into::into),
        is_flying_human: elem.posture() == Posture::Flying,
        hidden_outline_rgb: hidden_outline_rgb(ctx.host, entity),
    };

    let teleport_fade = teleport_fade(entity);
    if let Some((before, old_alpha, _new_alpha)) = teleport_fade {
        render_teleport_ghost(ctx, renderer, cached, facts, before, old_alpha);
    }

    // The teleport ghost above is masked independently at its old
    // position; this checkpoint applies current-position masks only
    // to the appearing sprite.
    let sprite_draw_checkpoint = renderer.draw_queue_checkpoint();

    // When the GoldenEye cheat is on, every PC sprite is
    // composited at 50% alpha (~128/255 in 8-bit).  Teleport
    // fade takes precedence — these are `else if` siblings.
    let (bank_id, variant, shadow_level) = (cached.bank_id, cached.variant, cached.shadow_level);
    if let Some((_, _, new_alpha)) = teleport_fade {
        renderer.render_cached_sprite_alpha(
            bank_id,
            variant,
            shadow_color,
            shadow_level,
            dst_rect,
            new_alpha,
        );
    } else if entity.is_pc() && ctx.engine.get_golden_eye_mode() {
        renderer.render_cached_sprite_alpha(
            bank_id,
            variant,
            shadow_color,
            shadow_level,
            dst_rect,
            128,
        );
    } else if uses_pixel_fog_visibility(entity) {
        renderer.render_cached_map_sprite(
            bank_id,
            variant,
            shadow_color,
            shadow_level,
            placement.smooth_world_origin,
            view,
            zoom,
            rotation,
        );
    } else {
        renderer.render_cached_sprite_subpixel(
            bank_id,
            variant,
            shadow_color,
            shadow_level,
            placement.screen_position.0,
            placement.screen_position.1,
            sw as f32 * zoom,
            sh as f32 * zoom,
        );
    }

    // ── Sprite occlusion masks ──
    //
    // After drawing the sprite, ask the grid for any building
    // masks that apply to this actor's position + layer, then
    // blit each mask's pre-composed background texture on top
    // of the sprite.  Where the mask is set the building
    // pixels reappear in front of the actor; elsewhere the
    // texture is transparent and the sprite stays visible.
    let sprite_world_bbox = engine_coordinates::MapBBox::from_coords(
        placement.world_origin.x,
        placement.world_origin.y,
        placement.world_origin.x + sw as f32,
        placement.world_origin.y + sh as f32,
    );
    // The mask pass is gated on `has_valid_box_for_masking`.
    // FX / target overlays never set the flag, so they render
    // without building-mask occlusion.  Flying humans use the
    // original projectile/flying-human mask path.
    if !kind.has_valid_box_for_masking() && !facts.is_flying_human {
        // Nothing more to do: sprite is drawn, no mask pass.
        return;
    }
    render_entity_sprite_masks(
        ctx,
        renderer,
        cached,
        facts,
        sprite_draw_checkpoint,
        dst_rect,
        &sprite_world_bbox,
    );
}

/// Outline colour restored on masked sprite edges while `draw_hidden` is on.
fn hidden_outline_rgb(host: &HostDraw<'_>, entity: &Entity) -> Option<(u8, u8, u8)> {
    if !host.frontend.input.feedback.draw_hidden {
        return None;
    }
    let elem = entity.element_data();
    // Ground objects always use Hidden; actors retain their active
    // targeting/parrying outline just like the original path.
    let color_565 = if matches!(
        entity.kind(),
        ElementKind::ObjectBonus | ElementKind::ObjectOther | ElementKind::ObjectScroll
    ) {
        elem.outline_colors[OutlineColorName::Hidden as usize]
    } else {
        elem.active_outline_color()
    };
    (color_565 != 0).then(|| rgb565_to_rgb8(color_565))
}

/// Cheat-teleport hulk-rebuild fade: `(position_before_teleport, ghost_alpha,
/// sprite_alpha)`.
///
/// When `teleport_counter > 0`, the PC is rendered TWICE: first
/// at `position_before_teleport` with alpha
/// `100 * counter / max_counter` (the vanishing ghost),
/// then at the current position with alpha
/// `100 - 100 * counter / max_counter` (the appearing
/// sprite).  As the counter ticks down 20→0 the ghost
/// fades out and the new sprite fades in.  The per-frame
/// decrement is done in `pre_render_engine_setup` via
/// `EngineInner::tick_pc_teleport_fades`.
fn teleport_fade(entity: &Entity) -> Option<(MapPoint, u8, u8)> {
    entity.pc_data().and_then(|pc| {
        if pc.teleport_counter > 0 && pc.max_teleport_counter > 0 {
            let ratio = pc.teleport_counter as f32 / pc.max_teleport_counter as f32;
            let old_alpha_255 = (ratio * 255.0).round().clamp(0.0, 255.0) as u8;
            let new_alpha_255 = ((1.0 - ratio) * 255.0).round().clamp(0.0, 255.0) as u8;
            Some((pc.position_before_teleport, old_alpha_255, new_alpha_255))
        } else {
            None
        }
    })
}

/// Render the vanishing ghost at the pre-teleport position first, so the
/// appearing sprite stacks on top. The ghost is masked at its own position.
fn render_teleport_ghost(
    ctx: &EntityPassCtx<'_, '_, '_>,
    renderer: &mut Renderer,
    cached: &CachedEntitySprite<'_>,
    facts: EntityMaskFacts,
    before: MapPoint,
    old_alpha: u8,
) {
    let EntityPassFrame {
        view,
        zoom,
        rotation,
        shadow_color,
        ..
    } = ctx.frame;
    let elem = cached.entity.element_data();
    let (sw, sh) = cached.size;
    let ghost = sprite_placement(
        before,
        elem.sprite.center,
        cached.script.offsets[cached.frame as usize],
        view,
        zoom,
        rotation,
    );
    let (ghost_dst_x, ghost_dst_y) = ghost.screen_origin;
    let ghost_x = ghost.world_origin.x;
    let ghost_y = ghost.world_origin.y;
    let ghost_rect = zoomed_sprite_rect(ghost_dst_x, ghost_dst_y, sw, sh, zoom);
    let ghost_draw_checkpoint = renderer.draw_queue_checkpoint();
    renderer.render_cached_sprite_alpha(
        cached.bank_id,
        cached.variant,
        shadow_color,
        cached.shadow_level,
        ghost_rect,
        old_alpha,
    );
    let ghost_world_bbox = engine_coordinates::MapBBox::from_coords(
        ghost_x,
        ghost_y,
        ghost_x + sw as f32,
        ghost_y + sh as f32,
    );
    let current_world = elem.position();
    let ghost_world = engine_coordinates::WorldPoint3D::new(
        before.x,
        before.y + current_world.z,
        current_world.z,
    );
    let ghost_mask_indices = applicable_sprite_masks(
        ctx.engine,
        ctx.assets,
        facts.actor_layer,
        &ghost_world_bbox,
        before,
        ghost_world,
        facts.is_flying_human,
        facts.is_flying_human,
    );
    let ghost_screen_masks =
        sprite_screen_masks(ctx.engine, &ghost_mask_indices, view, zoom, rotation);
    renderer.mask_queued_draws(ghost_draw_checkpoint, &ghost_screen_masks, ghost_rect);
    if let Some(rgb) = facts.hidden_outline_rgb {
        render_hidden_mask_outlines(ctx, renderer, cached, &ghost_screen_masks, ghost_rect, rgb);
    }
}

/// Building-occlusion mask pass for the sprite at its current position.
///
/// The mask lookup switches between
/// `get_masks_applied_to_character` and
/// `get_masks_applied_to_projectile` based on the masking
/// category.  PCs override to flying-human masking when
/// their posture is `Flying` so a PC mid-jump no longer
/// gets clipped by the building it's soaring over.  Arrows,
/// thrown bonuses and nets (`ElementKind::ObjectProjectile`
/// / `ObjectNet`) use the projectile masking category so
/// they route through the projectile polyline + 3D
/// altitude test, not the character polyline.
fn render_entity_sprite_masks(
    ctx: &EntityPassCtx<'_, '_, '_>,
    renderer: &mut Renderer,
    cached: &CachedEntitySprite<'_>,
    facts: EntityMaskFacts,
    sprite_draw_checkpoint: usize,
    dst_rect: Rect,
    sprite_world_bbox: &engine_coordinates::MapBBox,
) {
    let EntityPassFrame {
        view,
        zoom,
        rotation,
        ..
    } = ctx.frame;
    let entity = cached.entity;
    let actor_position = cached.world;
    let use_projectile_path = facts.is_flying_human || entity.kind().is_projectile();
    let projectile_mask_position =
        transition_crenel_climb_up_mask_position(entity, ctx.engine, ctx.assets)
            .unwrap_or_else(|| entity.element_data().position());
    let mask_indices = applicable_sprite_masks(
        ctx.engine,
        ctx.assets,
        facts.actor_layer,
        sprite_world_bbox,
        actor_position,
        projectile_mask_position,
        use_projectile_path,
        facts.is_flying_human,
    );
    // When `draw_hidden` is on, the original mutates the
    // temporary sprite surface per mask: masked pixels become
    // transparent, except horizontal transparent/body edges
    // become the actor's outline colour. Stencil rejection does the
    // transparency part; the hidden outline pass restores those edge
    // pixels.
    let screen_masks = sprite_screen_masks(ctx.engine, &mask_indices, view, zoom, rotation);
    if use_projectile_path {
        renderer.mask_queued_draws(sprite_draw_checkpoint, &screen_masks, dst_rect);
    } else {
        renderer.mask_queued_draws_with_depth(
            sprite_draw_checkpoint,
            &screen_masks,
            dst_rect,
            view.x,
            view.y,
            zoom,
            projectile_mask_position.y,
        );
    }

    if let Some(rgb) = facts.hidden_outline_rgb {
        render_hidden_mask_outlines(ctx, renderer, cached, &screen_masks, dst_rect, rgb);
    }
    if ctx.dev.debug.sprite_masks_display {
        render_sprite_mask_debug_overlay(
            ctx.host,
            ctx.engine,
            renderer,
            sprite_world_bbox,
            actor_position,
            projectile_mask_position,
            use_projectile_path,
            &mask_indices,
        );
    }
}

/// Restore the `draw_hidden` outline pixels on every mask covering `sprite_rect`.
fn render_hidden_mask_outlines(
    ctx: &EntityPassCtx<'_, '_, '_>,
    renderer: &mut Renderer,
    cached: &CachedEntitySprite<'_>,
    screen_masks: &[(u32, Rect)],
    sprite_rect: Rect,
    rgb: (u8, u8, u8),
) {
    for &(mask_idx, mask_rect) in screen_masks {
        let mask = &ctx.engine.fast_grid().level.masks[mask_idx as usize];
        renderer.render_hidden_mask_outline(
            ctx.host.frontend.resources.frame_holder(),
            cached.bank_id,
            cached.variant,
            ctx.frame.shadow_color,
            &mask.bitmap,
            mask.width,
            mask.height,
            mask_rect,
            sprite_rect,
            rgb,
        );
    }
}

/// Large patch animations are environment replacements, not point-sized
/// actors. Their anchor can sit behind the wall that their sprite closes, so
/// anchor visibility would discard the entire animation before the final fog
/// composite can clip it accurately per pixel.
pub(super) fn uses_pixel_fog_visibility(entity: &Entity) -> bool {
    matches!(entity, Entity::Fx(fx) if fx.fx.patch_index.is_some())
}

pub(super) fn transition_crenel_climb_up_mask_position(
    entity: &robin_engine::element::Entity,
    engine: &PresentationView<'_>,
    assets: &LevelAssets,
) -> Option<engine_coordinates::WorldPoint3D> {
    use robin_engine::order::OrderType;

    let elem = entity.element_data();
    if elem.sprite.last_action != OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
        || elem.sprite.current_frame != 0
    {
        return None;
    }
    let actor = entity.actor_data()?;
    if engine.installed_order_type(actor.installed_order?)
        != OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
    {
        return None;
    }
    let door_index = entity.position_iface().get_door()?;
    let door = engine.doors().get(usize::from(door_index))?;
    let point_mid = door.point_mid;
    let point_out = door.point_out;

    // The original game applies the high-crenel transition projection when the action finishes:
    // Set map position to the midpoint, obstacle/material to the exit projection
    // area, and old map position to the exit; then recompute all positions.
    // Frame 0 is visually still anchored at the pre-snap map point so its
    // offset lines up with frame 1, but the flying-human mask decision must
    // already use the far-side projection or the wall projectile masks erase
    // the whole frame.
    let door_sector_index = door.sector_out_index?;
    let mut best_z: Option<f32> = None;
    for obs in engine.sight_obstacles(assets).iter() {
        let Some(attachment) = obs.projection_area_ref() else {
            continue;
        };
        if attachment.layer.get() != door.layer_out
            || attachment.sector != door_sector_index
            || !obs.contains_point_projection(point_out)
        {
            continue;
        }
        let z = obs.compute_top_z(point_mid.x, point_mid.y);
        best_z = Some(best_z.map_or(z, |old| old.max(z)));
    }
    let z = best_z?;
    Some(engine_coordinates::WorldPoint3D {
        x: point_mid.x,
        y: point_mid.y + z,
        z,
    })
}

pub(super) fn render_sprite_mask_debug_overlay(
    host: &HostDraw<'_>,
    engine: &PresentationView<'_>,
    renderer: &mut Renderer,
    sprite_world_bbox: &engine_coordinates::MapBBox,
    actor_position: engine_coordinates::MapPoint,
    position_3d: engine_coordinates::WorldPoint3D,
    use_projectile_path: bool,
    mask_indices: &[engine_mask::MaskIndex],
) {
    if mask_indices.is_empty() && !use_projectile_path {
        return;
    }

    let sprite_color = if mask_indices.is_empty() {
        0x07ff
    } else {
        0xf81f
    };
    draw_map_bbox_outline(host, renderer, sprite_world_bbox, sprite_color);

    for &mask_idx in mask_indices {
        let mask = &engine.fast_grid().level.masks[usize::from(mask_idx)];
        draw_map_bbox_outline(host, renderer, &mask.bbox, 0xffe0);
    }

    draw_map_cross(host, renderer, actor_position, 0x07e0);
    if use_projectile_path {
        let projectile_test_point = position_3d.to_map();
        let viewport = host.viewport();
        let actor_screen = viewport.map_to_screen_unclamped(actor_position);
        let projectile_screen = viewport.map_to_screen_unclamped(projectile_test_point);
        renderer.draw_line_screen(
            actor_screen.x.round() as i32,
            actor_screen.y.round() as i32,
            projectile_screen.x.round() as i32,
            projectile_screen.y.round() as i32,
            0xfd20,
        );
        draw_map_cross(host, renderer, projectile_test_point, 0xfd20);
    }
}

pub(super) fn draw_map_bbox_outline(
    host: &HostDraw<'_>,
    renderer: &mut Renderer,
    bbox: &engine_coordinates::MapBBox,
    color: u16,
) {
    if !bbox.is_somewhere() {
        return;
    }
    let viewport = host.viewport();
    let min = viewport.map_to_screen_unclamped(engine_coordinates::MapPoint::new(
        bbox.x_min(),
        bbox.y_min(),
    ));
    let max = viewport.map_to_screen_unclamped(engine_coordinates::MapPoint::new(
        bbox.x_max(),
        bbox.y_max(),
    ));
    renderer.draw_rect_outline_screen(
        min.x.round() as i32,
        min.y.round() as i32,
        max.x.round() as i32,
        max.y.round() as i32,
        color,
    );
}

pub(super) fn draw_map_cross(
    host: &HostDraw<'_>,
    renderer: &mut Renderer,
    point: engine_coordinates::MapPoint,
    color: u16,
) {
    let screen = host.viewport().map_to_screen_unclamped(point);
    let (x, y) = (screen.x.round() as i32, screen.y.round() as i32);
    renderer.draw_line_screen(x - 4, y, x + 4, y, color);
    renderer.draw_line_screen(x, y - 4, x, y + 4, color);
}

// ─── GPU selection outline pass ──────────────────────────────────

/// Facts that decide whether an entity gets an outline and at what alpha.
/// Kept pure so the accessibility rules (constant alpha, fog wins, no
/// pulsing) can be checked without a renderer or a live engine.
#[derive(Debug, Clone, Copy)]
struct OutlineDecision {
    is_active: bool,
    hidden_in_building: bool,
    npc_in_building: bool,
    carried: bool,
    fog_visible: bool,
    is_player_controlled: bool,
    is_hostile: bool,
    reveal_hidden: bool,
    is_focused: bool,
    is_action_marked: bool,
    hulk_running: bool,
    hulk_level: u16,
}

/// Outline alpha percentage (0..=100), or `None` when no outline is drawn.
///
/// Inside a building only a directly controlled (green) character keeps a
/// constant-alpha outline; allied NPCs and blue-outlined troops need the
/// reveal key, as before. Outside the interior, roof/wall occlusion keeps
/// friendly troops outlined and now does the same for hostiles where fog facts
/// allow. Fog always takes priority. No pulsing (XAG 118).
fn outline_alpha_pct(d: OutlineDecision) -> Option<u16> {
    if !d.fog_visible {
        return None;
    }

    if d.hidden_in_building {
        if d.carried || (!d.is_player_controlled && !d.reveal_hidden) {
            return None;
        }
        return Some(100);
    }

    if !d.is_active {
        return None;
    }

    let occluded_outline = !d.carried && (d.npc_in_building || d.is_hostile);
    if !d.is_focused && !d.is_action_marked && !d.hulk_running && !occluded_outline {
        return None;
    }
    if d.is_focused || d.is_action_marked || occluded_outline {
        Some(100)
    } else {
        Some(d.hulk_level)
    }
}

/// Render coloured outlines for selected PCs and the hovered entity.
///
/// The selection-outline pass runs after all entity sprites are drawn
/// so the outline is drawn ON TOP of entities and is never occluded.
///
/// For each outlined entity, the cached outline mask texture is tinted and
/// alpha-modulated by the GPU pipeline (for hulk fade animation).
pub(crate) fn render_selection_outlines_gpu(
    host: &HostDraw<'_>,
    presentation: &FramePresentationInputs,
    engine: &PresentationView<'_>,
    renderer: &mut Renderer,
) {
    let view = presentation.view;
    let zoom = presentation.zoom;
    let rotation = presentation.rotation;
    let screen_w = presentation.screen_size.x as i32;
    let screen_h = presentation.screen_size.y as i32;
    let shadow_color = presentation.shadow_color;
    let shadow_level = host.frontend.resources.frame_holder().global_shadow();
    let apply_fog_to_all_sprites = presentation.graphic_config.apply_fog_to_all_sprites;

    for &entity_id in &presentation.draw_order_ids {
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        let elem = entity.element_data();

        // Only a directly controlled (green) character keeps a persistent
        // outline inside a building. Allied camp membership alone does not:
        // friendly NPCs there must need the reveal key like everyone else.
        let is_player_controlled = engine.hero_selection(host.local_seat).contains(&entity_id)
            || engine
                .tactical_selection(host.local_seat)
                .contains(&entity_id);

        // The selection set on its own does NOT draw the outline —
        // `refresh_pc_selection_hulk` seeds `running_hulk` on the first
        // frame of selection and it decays 100 -> 40 over `HULK_LENGTH`
        // frames. `is_focused` stands in for the mouse-hover mark and
        // `is_action_marked` covers the requirements-bar hover flag.
        let is_focused = host.frontend.input.feedback.focused_entity_id == Some(entity_id);
        let is_action_marked = host
            .frontend
            .input
            .feedback
            .marked_pc_ids
            .contains(&entity_id);
        let hulk_running = entity.human_data().is_some_and(|h| h.running_hulk > 0);

        let decision = OutlineDecision {
            is_active: entity.is_active(),
            hidden_in_building: elem.hidden_in_building,
            npc_in_building: npc_in_building(engine, entity),
            carried: elem.posture() == Posture::Carried,
            fog_visible: engine.fog_entity_visible(entity_id),
            is_player_controlled,
            is_hostile: engine.fog_entity_is_hostile(entity_id),
            reveal_hidden: host.frontend.input.feedback.reveal_hidden,
            is_focused,
            is_action_marked,
            hulk_running,
            hulk_level: entity.human_data().map(|h| h.hulk_level).unwrap_or(100),
        };
        let Some(alpha_pct) = outline_alpha_pct(decision) else {
            continue;
        };

        let variant = engine.resolve_render_variant_for_ambiance(
            entity,
            apply_fog_to_all_sprites,
            presentation.ambiance,
        );

        let is_selected_tactical = host.frontend.preferences().control_tactical_units()
            && engine
                .tactical_selection(host.local_seat)
                .contains(&entity_id);
        // TODO(XAG 102): the outline colour should be player-configurable
        // and verified at 4.5:1/3:1 against every in-game background.
        let outline_color_565 = if is_selected_tactical && hulk_running && !is_focused {
            robin_engine::element_kinds::outline_colors::pc_default()
        } else if is_focused || is_action_marked {
            elem.outline_colors[OutlineColorName::Default as usize]
        } else {
            elem.active_outline_color()
        };
        if outline_color_565 == 0 {
            continue;
        }

        // `alpha_pct` comes from `outline_alpha_pct`: a constant 100 for
        // focused/marked/indoor outlines, otherwise the native hulk fade.
        // Convert the 0-100 percentage to the renderer's 0-255 alpha range.
        let alpha_255 = ((alpha_pct as u32) * 255 / 100).min(255) as u8;

        // Resolve sprite frame (same calculation as render_entities_gpu).
        // See note there about `current_scripts_opt` vs direct field read.
        let sprite = &elem.sprite;
        let Some((script, frame, bank_id)) =
            current_render_frame(sprite, host.viewport().camera_steps())
        else {
            continue;
        };

        // Screen position. Use the same visual anchor as sprite rendering
        // so hover outlines stay aligned with targets and airborne actors.
        let visual_pos = entity_visual_map_position(entity);
        let world_x = visual_pos.x;
        let world_y = visual_pos.y;
        let screen = rotation.to_screen(MapPoint::new(world_x, world_y), view, zoom);
        let screen_x = screen.x as i32;
        let screen_y = screen.y as i32;
        if outside_sprite_cull_margin((screen_x, screen_y), (screen_w, screen_h), zoom) {
            continue;
        }

        // Sprite-position calculation (same as render_entities_gpu).
        let placement = sprite_placement(
            MapPoint::new(world_x, world_y),
            sprite.center,
            script.offsets[frame as usize],
            view,
            zoom,
            rotation,
        );
        let (dst_x, dst_y) = placement.screen_origin;

        if let Some((ow, oh)) = renderer.ensure_outline_cached(
            host.frontend.resources.frame_holder(),
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        ) {
            let rgb = rgb565_to_rgb8(outline_color_565);
            let outline_x = dst_x - (OUTLINE_PAD as f32 * zoom).round() as i32;
            let outline_y = dst_y;
            let outline_rect = zoomed_sprite_rect(outline_x, outline_y, ow, oh, zoom);
            renderer.render_cached_outline(
                bank_id,
                variant,
                shadow_color,
                shadow_level,
                outline_rect,
                rgb,
                alpha_255,
            );
        }
    }
}

/// Fallback: draw a colored rectangle for entities without sprites.
pub(super) fn render_entity_fallback(
    renderer: &mut Renderer,
    kind: robin_engine::element::ElementKind,
    screen_x: i32,
    screen_y: i32,
    screen_w: i32,
    screen_h: i32,
) {
    use robin_engine::element::ElementKind;

    let (r, g, b): (u8, u8, u8) = match kind {
        ElementKind::ActorPc => (0, 255, 0),
        ElementKind::ActorSoldier => (255, 0, 0),
        ElementKind::ActorCivilian => (0, 0, 255),
        ElementKind::Fx => (255, 224, 0),
        ElementKind::Target => (255, 0, 255),
        ElementKind::ObjectBonus => (0, 255, 255),
        _ => (255, 255, 255),
    };

    let half = 4;
    let x = (screen_x - half).max(0);
    let y = (screen_y - half).max(0);
    let w = ((screen_x + half).min(screen_w) - x).max(0);
    let h = ((screen_y + half).min(screen_h) - y).max(0);
    if w > 0 && h > 0 {
        renderer.render_gpu_rect(x, y, w, h, [r, g, b, 255]);
    }
}

// ─── Background animation rendering ──────────────────────────────────

/// Render background animations (elevation-0 FX) as GPU sprites.
///
/// Iterates the background-animations list and renders them BEFORE
/// the main entity loop.  Background animations are excluded from
/// `display_order` by `sort_for_display`, so we render them in a
/// dedicated pass here.
///
/// Must be called after `flush_base_layer` (GPU phase active) and before
/// `render_entities_gpu`.
pub(crate) fn render_bg_animations_gpu(
    engine: &PresentationView<'_>,
    host: &HostDraw<'_>,
    presentation: &FramePresentationInputs,
    renderer: &mut Renderer,
) {
    let mut bg_animation_ids = engine.bg_animation_ids().peekable();
    if bg_animation_ids.peek().is_none() {
        return;
    }
    render_fx_entities_gpu(bg_animation_ids, engine, host, presentation, renderer);
}

pub(super) fn render_fx_entities_gpu<I>(
    entity_ids: I,
    engine: &PresentationView<'_>,
    host: &HostDraw<'_>,
    presentation: &FramePresentationInputs,
    renderer: &mut Renderer,
) where
    I: IntoIterator<Item = engine_element::EntityId>,
{
    let view = host.viewport().view_position;
    let zoom = host.viewport().zoom_factor;
    let rotation = presentation.rotation;
    let screen_w = host.viewport().screen_size.x as i32;
    let screen_h = host.viewport().screen_size.y as i32;
    let shadow_color = presentation.shadow_color;
    let global_shadow = host.frontend.resources.frame_holder().global_shadow();

    // Bg animations are unforced ground-level non-masked FX, so they
    // are suppressed when the player has disabled "Display Animations"
    // unless `force_display` or `patch_index` overrides.  See
    // `render_entities_gpu` for the full gate; identical logic via
    // `Entity::is_to_be_displayed`.
    let graphic_config = &presentation.graphic_config;
    let display_anim = graphic_config.display_anim;
    let apply_fog_to_all_sprites = graphic_config.apply_fog_to_all_sprites;

    for entity_id in entity_ids {
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        if !entity.is_active() {
            continue;
        }
        if !entity.is_to_be_displayed(display_anim) {
            continue;
        }
        let variant = engine.resolve_render_variant_for_ambiance(
            entity,
            apply_fog_to_all_sprites,
            presentation.ambiance,
        );

        let elem = entity.element_data();
        let sprite = &elem.sprite;
        let Some((script, frame, bank_id)) =
            current_render_frame(sprite, host.viewport().camera_steps())
        else {
            continue;
        };

        let world_x = elem.position_map().x;
        let world_y = elem.position_map().y;
        if world_x == 0.0 && world_y == 0.0 {
            continue;
        }

        let screen = rotation.to_screen(MapPoint::new(world_x, world_y), view, zoom);
        let screen_x = screen.x as i32;
        let screen_y = screen.y as i32;
        if outside_sprite_cull_margin((screen_x, screen_y), (screen_w, screen_h), zoom) {
            continue;
        }

        // FX entities composite a shadow when `rendering_properties`
        // is `NeedShadow`, and skip it for `Blocky`.  Zero
        // `shadow_level` for `Blocky` FX.
        let shadow_level = match entity.fx_data() {
            Some(fx) if fx.rendering_properties == RenderingProperties::Blocky => 0,
            _ => global_shadow,
        };

        if let Some((sw, sh)) = renderer.ensure_sprite_cached(
            host.frontend.resources.frame_holder(),
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        ) {
            let placement = sprite_placement(
                MapPoint::new(world_x, world_y),
                sprite.center,
                script.offsets[frame as usize],
                view,
                zoom,
                rotation,
            );
            if uses_pixel_fog_visibility(entity) {
                renderer.render_cached_map_sprite(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    placement.smooth_world_origin,
                    view,
                    zoom,
                    rotation,
                );
            } else {
                renderer.render_cached_sprite_subpixel(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    placement.screen_position.0,
                    placement.screen_position.1,
                    sw as f32 * zoom,
                    sh as f32 * zoom,
                );
            }
        }
    }
}

#[cfg(test)]
mod outline_decision_tests {
    use super::*;

    fn decision() -> OutlineDecision {
        OutlineDecision {
            is_active: true,
            hidden_in_building: false,
            npc_in_building: false,
            carried: false,
            fog_visible: true,
            is_player_controlled: false,
            is_hostile: false,
            reveal_hidden: false,
            is_focused: false,
            is_action_marked: false,
            hulk_running: false,
            hulk_level: 100,
        }
    }

    #[test]
    fn controlled_hidden_character_gets_a_constant_alpha_outline() {
        let mut d = decision();
        d.hidden_in_building = true;
        d.is_active = false;
        d.is_player_controlled = true;
        assert_eq!(outline_alpha_pct(d), Some(100));

        // Without either condition the hidden character stays invisible.
        let mut uncontrolled = d;
        uncontrolled.is_player_controlled = false;
        assert_eq!(outline_alpha_pct(uncontrolled), None);
    }

    #[test]
    fn hostile_hidden_character_is_outlined_only_after_reveal() {
        let mut d = decision();
        d.hidden_in_building = true;
        d.is_active = false;
        assert_eq!(outline_alpha_pct(d), None);

        d.reveal_hidden = true;
        assert_eq!(outline_alpha_pct(d), Some(100));
    }

    #[test]
    fn fog_hidden_character_has_no_outline_even_when_revealed() {
        let mut d = decision();
        d.hidden_in_building = true;
        d.is_active = false;
        d.reveal_hidden = true;
        d.fog_visible = false;
        assert_eq!(outline_alpha_pct(d), None);
    }

    #[test]
    fn carried_hidden_body_never_gets_an_outline() {
        let mut d = decision();
        d.hidden_in_building = true;
        d.is_player_controlled = true;
        d.carried = true;
        assert_eq!(outline_alpha_pct(d), None);
    }

    #[test]
    fn allied_non_controlled_hidden_character_has_no_persistent_outline() {
        let mut d = decision();
        d.hidden_in_building = true;
        // Allied camp membership is not player control: this is a blue NPC.
        d.is_player_controlled = false;
        d.reveal_hidden = false;
        assert_eq!(
            outline_alpha_pct(d),
            None,
            "inside a building only green controlled characters keep the new outline"
        );
        d.reveal_hidden = true;
        assert_eq!(
            outline_alpha_pct(d),
            Some(100),
            "allied NPCs behave as before once the reveal key is held"
        );
    }

    #[test]
    fn friendly_troop_inside_a_building_is_not_outlined() {
        let mut d = decision();
        d.hidden_in_building = true; // authoritative interior state
        d.npc_in_building = true; // the sector is a building
        d.is_player_controlled = false; // blue/friendly, not a green PC
        assert_eq!(outline_alpha_pct(d), None, "inside is hidden, not outlined");
    }

    #[test]
    fn friendly_troop_occluded_outside_keeps_its_outline() {
        let mut d = decision();
        d.npc_in_building = true; // outside the interior, but roof/wall occludes
        d.is_player_controlled = false;
        assert_eq!(
            outline_alpha_pct(d),
            Some(100),
            "the useful blue occlusion case must survive the inside-building fix"
        );
    }

    #[test]
    fn hostile_troop_occluded_outside_gets_an_outline() {
        let mut d = decision();
        d.is_hostile = true;
        assert_eq!(
            outline_alpha_pct(d),
            Some(100),
            "red troops behind walls are shown where fog facts allow"
        );
    }
}

// ─── Dotted outlines of ground hidden behind buildings ───────────

use geo::{Area as _, Intersects as _};

/// Ground region occluded by one opaque obstacle volume, in map coordinates.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HiddenGroundZone {
    polygon: geo::Polygon<f32>,
}

/// Distance between dots and dot thickness in map units.
const HIDDEN_ZONE_DOT_SPACING: f32 = 24.0;
const HIDDEN_ZONE_DOT_THICKNESS: f32 = 1.5;
const HIDDEN_ZONE_COLOR: u32 = 0x7F_DF;
const MIN_HIDDEN_ZONE_AREA: f32 = 1.0;

impl HiddenGroundZone {
    /// Closed exterior boundary; ground points project to themselves.
    fn boundary(&self) -> Vec<MapPoint> {
        self.polygon
            .exterior()
            .0
            .iter()
            .map(|coord| MapPoint::new(coord.x, coord.y))
            .collect()
    }

    fn contains_feet(&self, feet: GroundPoint) -> bool {
        self.polygon.intersects(&geo::Point::new(feet.x, feet.y))
    }
}

/// The walkable-floor boundary is the obstacle's ground footprint: the line
/// where a character stops being able to stand. `obstacle.polygon` already
/// lives in ground coordinates, the same space as character positions. The
/// old projection sweep mixed `(x, y - z)` roof points with ground points and
/// subtracted one from the other, which traced building tops across the map.
fn zone_for_obstacle(obstacle: &engine_sight_obstacle::SightObstacle) -> Option<HiddenGroundZone> {
    if obstacle.is_projection_area() || !obstacle.is_opaque() || !obstacle.is_solid() {
        return None;
    }
    // A zero-height volume is floor geometry, not a building hiding ground.
    if !obstacle
        .obstacle_points
        .iter()
        .any(|point| point.z_top > point.z_bottom)
    {
        return None;
    }
    let footprint = obstacle.polygon.as_geo();
    if footprint.exterior().0.len() < 3 || footprint.unsigned_area() < MIN_HIDDEN_ZONE_AREA {
        return None;
    }
    Some(HiddenGroundZone {
        polygon: footprint.clone(),
    })
}

/// Derive every occluded ground region from the active opaque obstacles.
pub(crate) fn derive_hidden_ground_zones(
    obstacles: &engine_sight_obstacle::ObstacleList<'_>,
) -> Vec<HiddenGroundZone> {
    let mut zones = Vec::new();
    for (index, obstacle) in obstacles.iter_indexed() {
        if !obstacles.is_active(index as usize) {
            continue;
        }
        if let Some(zone) = zone_for_obstacle(obstacle) {
            zones.push(zone);
        }
    }
    zones
}

/// Indices to reveal: all of them for the whole-map override, otherwise only
/// zones containing at least one local player character's feet. Callers must
/// pass those feet only — NPC positions elsewhere on the map are irrelevant.
pub(crate) fn visible_hidden_zone_indices(
    zones: &[HiddenGroundZone],
    feet: &[GroundPoint],
    reveal_all: bool,
) -> Vec<usize> {
    if reveal_all {
        return (0..zones.len()).collect();
    }
    (0..zones.len())
        .filter(|&index| feet.iter().any(|&foot| zones[index].contains_feet(foot)))
        .collect()
}

/// Closed segment loops for the given zones, for a render pass or a test.
pub(crate) fn hidden_zone_outline_segments(
    zones: &[HiddenGroundZone],
    indices: &[usize],
) -> Vec<(MapPoint, MapPoint)> {
    let mut segments = Vec::new();
    for &index in indices {
        // Missing zone: skip it rather than drawing a made-up boundary.
        let Some(zone) = zones.get(index) else {
            continue;
        };
        let boundary = zone.boundary();
        for edge in boundary.windows(2) {
            segments.push((edge[0], edge[1]));
        }
        if let (Some(first), Some(last)) = (boundary.first(), boundary.last())
            && first != last
        {
            segments.push((*last, *first));
        }
    }
    segments
}

/// Draw this frame's dotted zone boundaries. Every input is read fresh, so
/// releasing the override or moving the last actor out removes the lines on
/// the same frame; nothing is latched into presentation state.
fn render_hidden_ground_outlines(
    host: &HostDraw<'_>,
    engine: &PresentationView<'_>,
    assets: &robin_engine::engine::LevelAssets,
    dev: &robin_engine::engine::DevState,
    renderer: &mut Renderer,
) {
    let obstacles = engine.sight_obstacles(assets);
    let zones = derive_hidden_ground_zones(&obstacles);
    if zones.is_empty() {
        return;
    }
    // Only the local player's characters make a zone relevant. NPCs occupy
    // buildings all over a castle map and made every zone visible every frame.
    let feet: Vec<GroundPoint> = engine
        .pc_ids()
        .iter()
        .filter_map(|&id| engine.get_entity(id))
        .filter(|entity| entity.is_active())
        .map(|entity| {
            let position = entity.element_data().position();
            GroundPoint::new(position.x, position.y)
        })
        .collect();
    let shown = visible_hidden_zone_indices(&zones, &feet, dev.debug.hidden_areas_all);
    if shown.is_empty() {
        return;
    }
    let draw_manager = host.draw_manager();
    let color = draw_manager.pack_color(HIDDEN_ZONE_COLOR);
    let mut phase = 0.0;
    let mut previous_end = None;
    for edge in hidden_zone_outline_segments(&zones, &shown) {
        if previous_end != Some(edge.0) {
            phase = 0.0;
        }
        draw_manager.draw_dotted_line(
            renderer,
            edge.0,
            edge.1,
            &mut phase,
            HIDDEN_ZONE_DOT_SPACING,
            HIDDEN_ZONE_DOT_THICKNESS,
            color,
        );
        previous_end = Some(edge.1);
    }
}

#[cfg(test)]
mod hidden_ground_tests {
    use super::*;
    use engine_sight_obstacle::{
        ObstaclePoint, SIGHTOBSTACLE_OPAQUE, SIGHTOBSTACLE_PROJECTION_AREA, SIGHTOBSTACLE_SOLID,
        SightObstacle,
    };

    fn prism(id: u32, min_x: f32, min_y: f32, z_top: f32, flags: u32) -> SightObstacle {
        let corners = [
            (min_x, min_y),
            (min_x + 10.0, min_y),
            (min_x + 10.0, min_y + 10.0),
            (min_x, min_y + 10.0),
        ];
        let mut obstacle = SightObstacle::new(id, flags);
        obstacle.obstacle_points = corners
            .iter()
            .map(|&(x, y)| ObstaclePoint {
                x,
                y,
                z_top,
                z_bottom: 0.0,
            })
            .collect();
        obstacle.rebuild_geometry();
        obstacle
    }

    fn building(id: u32, min_x: f32, min_y: f32, z_top: f32) -> SightObstacle {
        prism(
            id,
            min_x,
            min_y,
            z_top,
            SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE,
        )
    }

    fn one_building_zones() -> Vec<HiddenGroundZone> {
        let obstacles = [building(1, 0.0, 0.0, 5.0)];
        let list = engine_sight_obstacle::ObstacleList::from_slice_all_active(&obstacles);
        derive_hidden_ground_zones(&list)
    }

    fn zone_area(zone: &HiddenGroundZone) -> f32 {
        zone.polygon.unsigned_area()
    }

    #[test]
    fn hidden_zone_is_the_walkable_floor_boundary_not_the_building_top() {
        let zones = one_building_zones();
        assert_eq!(zones.len(), 1, "one opaque building yields one hidden zone");
        assert!(
            (zone_area(&zones[0]) - 100.0).abs() < 0.01,
            "the boundary of a 10x10 unwalkable footprint encloses 100 map units: {}",
            zone_area(&zones[0])
        );
        assert!(
            zones[0].contains_feet(GroundPoint::new(5.0, 5.0)),
            "the zone covers the ground a character cannot stand on"
        );
        assert!(
            !zones[0].contains_feet(GroundPoint::new(5.0, -2.0)),
            "the projection of the roof is not a walkable-floor boundary"
        );
    }

    #[test]
    fn only_opaque_solid_obstacles_have_hidden_ground_zones() {
        let solid = building(1, 0.0, 0.0, 5.0);
        let transparent = prism(2, 20.0, 0.0, 5.0, SIGHTOBSTACLE_SOLID);
        let mut projection_area = building(3, 40.0, 0.0, 5.0);
        projection_area.obstacle_type |= SIGHTOBSTACLE_PROJECTION_AREA;
        let non_solid = prism(4, 60.0, 0.0, 5.0, SIGHTOBSTACLE_OPAQUE);
        let obstacles = [solid, transparent, projection_area, non_solid];
        let list = engine_sight_obstacle::ObstacleList::from_slice_all_active(&obstacles);
        assert_eq!(
            derive_hidden_ground_zones(&list).len(),
            1,
            "a floor-marker, a transparent volume and a non-solid volume mark no unwalkable floor"
        );
    }

    #[test]
    fn flat_and_inactive_volumes_invent_no_hidden_zone() {
        let flat = building(1, 0.0, 0.0, 0.0);
        let building_2 = building(2, 40.0, 0.0, 5.0);
        let obstacles = [flat, building_2];
        let list = engine_sight_obstacle::ObstacleList {
            static_obstacles: &obstacles,
            dynamic_obstacles: &[],
            static_active: &[true, false],
        };
        assert!(derive_hidden_ground_zones(&list).is_empty());
    }

    #[test]
    fn ground_zone_visibility_follows_the_pc_inside_the_footprint() {
        let zones = one_building_zones();
        assert_eq!(
            visible_hidden_zone_indices(&zones, &[GroundPoint::new(5.0, 5.0)], false),
            vec![0],
            "a player character under the building makes its floor boundary visible"
        );
        assert_eq!(
            visible_hidden_zone_indices(&zones, &[GroundPoint::new(5.0, -2.0)], false),
            Vec::<usize>::new(),
            "standing on the walkable side draws no line"
        );
    }

    #[test]
    fn reveal_all_is_a_superset_of_zone_occupancy() {
        let obstacles = [building(1, 0.0, 0.0, 5.0), building(2, 40.0, 0.0, 5.0)];
        let list = engine_sight_obstacle::ObstacleList::from_slice_all_active(&obstacles);
        let zones = derive_hidden_ground_zones(&list);
        assert_eq!(zones.len(), 2);
        assert_eq!(
            visible_hidden_zone_indices(&zones, &[], false),
            Vec::<usize>::new()
        );
        assert_eq!(visible_hidden_zone_indices(&zones, &[], true), vec![0, 1]);
        assert_eq!(
            visible_hidden_zone_indices(&zones, &[GroundPoint::new(5.0, 5.0)], false),
            vec![0]
        );
        assert_eq!(
            visible_hidden_zone_indices(
                &zones,
                &[GroundPoint::new(45.0, 5.0), GroundPoint::new(5.0, 5.0)],
                false,
            ),
            vec![0, 1]
        );
    }

    #[test]
    fn one_rectangular_zone_draws_four_floor_edges() {
        let zones = one_building_zones();
        let segments = hidden_zone_outline_segments(&zones, &[0]);
        assert_eq!(
            segments.len(),
            4,
            "a rectangular footprint has four boundary edges, not a roof silhouette"
        );
        assert_eq!(segments.first().map(|s| s.0), segments.last().map(|s| s.1));
        assert!(hidden_zone_outline_segments(&zones, &[]).is_empty());
    }

    #[test]
    fn dot_spacing_is_restrained() {
        const {
            assert!(
                HIDDEN_ZONE_DOT_SPACING >= 20.0,
                "dots must not carpet the whole map: spacing is {HIDDEN_ZONE_DOT_SPACING}"
            );
        };
    }

    #[test]
    fn leaving_the_floor_zone_removes_lines_the_same_frame() {
        let zones = one_building_zones();
        let inside = [GroundPoint::new(5.0, 5.0)];
        let outside = [GroundPoint::new(5.0, -2.0)];

        assert_eq!(visible_hidden_zone_indices(&zones, &inside, false), vec![0]);
        assert_eq!(hidden_zone_outline_segments(&zones, &[0]).len(), 4);
        assert_eq!(
            visible_hidden_zone_indices(&zones, &outside, false),
            Vec::<usize>::new(),
            "feet leaving the zone remove the outline immediately"
        );
        assert_eq!(
            hidden_zone_outline_segments(
                &zones,
                &visible_hidden_zone_indices(&zones, &outside, false),
            ),
            Vec::<(MapPoint, MapPoint)>::new()
        );
    }
}
