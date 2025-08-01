use bevy_ecs::{
    entity::{EntityHashMap, EntityHashSet},
    prelude::*,
};
use bevy_math::{ops, Mat4, Vec3A, Vec4};
use bevy_reflect::prelude::*;
use bevy_render::{
    camera::{Camera, Projection},
    extract_component::ExtractComponent,
    extract_resource::ExtractResource,
    mesh::Mesh3d,
    primitives::{Aabb, CascadesFrusta, CubemapFrusta, Frustum, Sphere},
    view::{
        InheritedVisibility, NoFrustumCulling, PreviousVisibleEntities, RenderLayers,
        ViewVisibility, VisibilityClass, VisibilityRange, VisibleEntityRanges,
    },
};
use bevy_transform::components::{GlobalTransform, Transform};
use bevy_utils::Parallel;
use core::{marker::PhantomData, ops::DerefMut};

use crate::*;

mod ambient_light;
pub use ambient_light::AmbientLight;

mod point_light;
pub use point_light::PointLight;
mod spot_light;
pub use spot_light::SpotLight;
mod directional_light;
pub use directional_light::DirectionalLight;

/// Constants for operating with the light units: lumens, and lux.
pub mod light_consts {
    /// Approximations for converting the wattage of lamps to lumens.
    ///
    /// The **lumen** (symbol: **lm**) is the unit of [luminous flux], a measure
    /// of the total quantity of [visible light] emitted by a source per unit of
    /// time, in the [International System of Units] (SI).
    ///
    /// For more information, see [wikipedia](https://en.wikipedia.org/wiki/Lumen_(unit))
    ///
    /// [luminous flux]: https://en.wikipedia.org/wiki/Luminous_flux
    /// [visible light]: https://en.wikipedia.org/wiki/Visible_light
    /// [International System of Units]: https://en.wikipedia.org/wiki/International_System_of_Units
    pub mod lumens {
        pub const LUMENS_PER_LED_WATTS: f32 = 90.0;
        pub const LUMENS_PER_INCANDESCENT_WATTS: f32 = 13.8;
        pub const LUMENS_PER_HALOGEN_WATTS: f32 = 19.8;
    }

    /// Predefined for lux values in several locations.
    ///
    /// The **lux** (symbol: **lx**) is the unit of [illuminance], or [luminous flux] per unit area,
    /// in the [International System of Units] (SI). It is equal to one lumen per square meter.
    ///
    /// For more information, see [wikipedia](https://en.wikipedia.org/wiki/Lux)
    ///
    /// [illuminance]: https://en.wikipedia.org/wiki/Illuminance
    /// [luminous flux]: https://en.wikipedia.org/wiki/Luminous_flux
    /// [International System of Units]: https://en.wikipedia.org/wiki/International_System_of_Units
    pub mod lux {
        /// The amount of light (lux) in a moonless, overcast night sky. (starlight)
        pub const MOONLESS_NIGHT: f32 = 0.0001;
        /// The amount of light (lux) during a full moon on a clear night.
        pub const FULL_MOON_NIGHT: f32 = 0.05;
        /// The amount of light (lux) during the dark limit of civil twilight under a clear sky.
        pub const CIVIL_TWILIGHT: f32 = 3.4;
        /// The amount of light (lux) in family living room lights.
        pub const LIVING_ROOM: f32 = 50.;
        /// The amount of light (lux) in an office building's hallway/toilet lighting.
        pub const HALLWAY: f32 = 80.;
        /// The amount of light (lux) in very dark overcast day
        pub const DARK_OVERCAST_DAY: f32 = 100.;
        /// The amount of light (lux) in an office.
        pub const OFFICE: f32 = 320.;
        /// The amount of light (lux) during sunrise or sunset on a clear day.
        pub const CLEAR_SUNRISE: f32 = 400.;
        /// The amount of light (lux) on an overcast day; typical TV studio lighting
        pub const OVERCAST_DAY: f32 = 1000.;
        /// The amount of light (lux) from ambient daylight (not direct sunlight).
        pub const AMBIENT_DAYLIGHT: f32 = 10_000.;
        /// The amount of light (lux) in full daylight (not direct sun).
        pub const FULL_DAYLIGHT: f32 = 20_000.;
        /// The amount of light (lux) in direct sunlight.
        pub const DIRECT_SUNLIGHT: f32 = 100_000.;
        /// The amount of light (lux) of raw sunlight, not filtered by the atmosphere.
        pub const RAW_SUNLIGHT: f32 = 130_000.;
    }
}

/// Marker resource for whether shadows are enabled for this material type
#[derive(Resource, Debug)]
pub struct ShadowsEnabled<M: Material>(PhantomData<M>);

impl<M: Material> Default for ShadowsEnabled<M> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

/// Controls the resolution of [`PointLight`] shadow maps.
///
/// ```
/// # use bevy_app::prelude::*;
/// # use bevy_pbr::PointLightShadowMap;
/// App::new()
///     .insert_resource(PointLightShadowMap { size: 2048 });
/// ```
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource, Debug, Default, Clone)]
pub struct PointLightShadowMap {
    /// The width and height of each of the 6 faces of the cubemap.
    ///
    /// Defaults to `1024`.
    pub size: usize,
}

impl Default for PointLightShadowMap {
    fn default() -> Self {
        Self { size: 1024 }
    }
}

/// A convenient alias for `Or<(With<PointLight>, With<SpotLight>,
/// With<DirectionalLight>)>`, for use with [`bevy_render::view::VisibleEntities`].
pub type WithLight = Or<(With<PointLight>, With<SpotLight>, With<DirectionalLight>)>;

/// Controls the resolution of [`DirectionalLight`] shadow maps.
///
/// ```
/// # use bevy_app::prelude::*;
/// # use bevy_pbr::DirectionalLightShadowMap;
/// App::new()
///     .insert_resource(DirectionalLightShadowMap { size: 4096 });
/// ```
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource, Debug, Default, Clone)]
pub struct DirectionalLightShadowMap {
    // The width and height of each cascade.
    ///
    /// Defaults to `2048`.
    pub size: usize,
}

impl Default for DirectionalLightShadowMap {
    fn default() -> Self {
        Self { size: 2048 }
    }
}

/// Controls how cascaded shadow mapping works.
/// Prefer using [`CascadeShadowConfigBuilder`] to construct an instance.
///
/// ```
/// # use bevy_pbr::CascadeShadowConfig;
/// # use bevy_pbr::CascadeShadowConfigBuilder;
/// # use bevy_utils::default;
/// #
/// let config: CascadeShadowConfig = CascadeShadowConfigBuilder {
///   maximum_distance: 100.0,
///   ..default()
/// }.into();
/// ```
#[derive(Component, Clone, Debug, Reflect)]
#[reflect(Component, Default, Debug, Clone)]
pub struct CascadeShadowConfig {
    /// The (positive) distance to the far boundary of each cascade.
    pub bounds: Vec<f32>,
    /// The proportion of overlap each cascade has with the previous cascade.
    pub overlap_proportion: f32,
    /// The (positive) distance to the near boundary of the first cascade.
    pub minimum_distance: f32,
}

impl Default for CascadeShadowConfig {
    fn default() -> Self {
        CascadeShadowConfigBuilder::default().into()
    }
}

fn calculate_cascade_bounds(
    num_cascades: usize,
    nearest_bound: f32,
    shadow_maximum_distance: f32,
) -> Vec<f32> {
    if num_cascades == 1 {
        return vec![shadow_maximum_distance];
    }
    let base = ops::powf(
        shadow_maximum_distance / nearest_bound,
        1.0 / (num_cascades - 1) as f32,
    );
    (0..num_cascades)
        .map(|i| nearest_bound * ops::powf(base, i as f32))
        .collect()
}

/// Builder for [`CascadeShadowConfig`].
pub struct CascadeShadowConfigBuilder {
    /// The number of shadow cascades.
    /// More cascades increases shadow quality by mitigating perspective aliasing - a phenomenon where areas
    /// nearer the camera are covered by fewer shadow map texels than areas further from the camera, causing
    /// blocky looking shadows.
    ///
    /// This does come at the cost increased rendering overhead, however this overhead is still less
    /// than if you were to use fewer cascades and much larger shadow map textures to achieve the
    /// same quality level.
    ///
    /// In case rendered geometry covers a relatively narrow and static depth relative to camera, it may
    /// make more sense to use fewer cascades and a higher resolution shadow map texture as perspective aliasing
    /// is not as much an issue. Be sure to adjust `minimum_distance` and `maximum_distance` appropriately.
    pub num_cascades: usize,
    /// The minimum shadow distance, which can help improve the texel resolution of the first cascade.
    /// Areas nearer to the camera than this will likely receive no shadows.
    ///
    /// NOTE: Due to implementation details, this usually does not impact shadow quality as much as
    /// `first_cascade_far_bound` and `maximum_distance`. At many view frustum field-of-views, the
    /// texel resolution of the first cascade is dominated by the width / height of the view frustum plane
    /// at `first_cascade_far_bound` rather than the depth of the frustum from `minimum_distance` to
    /// `first_cascade_far_bound`.
    pub minimum_distance: f32,
    /// The maximum shadow distance.
    /// Areas further from the camera than this will likely receive no shadows.
    pub maximum_distance: f32,
    /// Sets the far bound of the first cascade, relative to the view origin.
    /// In-between cascades will be exponentially spaced relative to the maximum shadow distance.
    /// NOTE: This is ignored if there is only one cascade, the maximum distance takes precedence.
    pub first_cascade_far_bound: f32,
    /// Sets the overlap proportion between cascades.
    /// The overlap is used to make the transition from one cascade's shadow map to the next
    /// less abrupt by blending between both shadow maps.
    pub overlap_proportion: f32,
}

impl CascadeShadowConfigBuilder {
    /// Returns the cascade config as specified by this builder.
    pub fn build(&self) -> CascadeShadowConfig {
        assert!(
            self.num_cascades > 0,
            "num_cascades must be positive, but was {}",
            self.num_cascades
        );
        assert!(
            self.minimum_distance >= 0.0,
            "maximum_distance must be non-negative, but was {}",
            self.minimum_distance
        );
        assert!(
            self.num_cascades == 1 || self.minimum_distance < self.first_cascade_far_bound,
            "minimum_distance must be less than first_cascade_far_bound, but was {}",
            self.minimum_distance
        );
        assert!(
            self.maximum_distance > self.minimum_distance,
            "maximum_distance must be greater than minimum_distance, but was {}",
            self.maximum_distance
        );
        assert!(
            (0.0..1.0).contains(&self.overlap_proportion),
            "overlap_proportion must be in [0.0, 1.0) but was {}",
            self.overlap_proportion
        );
        CascadeShadowConfig {
            bounds: calculate_cascade_bounds(
                self.num_cascades,
                self.first_cascade_far_bound,
                self.maximum_distance,
            ),
            overlap_proportion: self.overlap_proportion,
            minimum_distance: self.minimum_distance,
        }
    }
}

impl Default for CascadeShadowConfigBuilder {
    fn default() -> Self {
        // The defaults are chosen to be similar to be Unity, Unreal, and Godot.
        // Unity: first cascade far bound = 10.05, maximum distance = 150.0
        // Unreal Engine 5: maximum distance = 200.0
        // Godot: first cascade far bound = 10.0, maximum distance = 100.0
        Self {
            // Currently only support one cascade in WebGL 2.
            num_cascades: if cfg!(all(
                feature = "webgl",
                target_arch = "wasm32",
                not(feature = "webgpu")
            )) {
                1
            } else {
                4
            },
            minimum_distance: 0.1,
            maximum_distance: 150.0,
            first_cascade_far_bound: 10.0,
            overlap_proportion: 0.2,
        }
    }
}

impl From<CascadeShadowConfigBuilder> for CascadeShadowConfig {
    fn from(builder: CascadeShadowConfigBuilder) -> Self {
        builder.build()
    }
}

#[derive(Component, Clone, Debug, Default, Reflect)]
#[reflect(Component, Debug, Default, Clone)]
pub struct Cascades {
    /// Map from a view to the configuration of each of its [`Cascade`]s.
    pub cascades: EntityHashMap<Vec<Cascade>>,
}

#[derive(Clone, Debug, Default, Reflect)]
#[reflect(Clone, Default)]
pub struct Cascade {
    /// The transform of the light, i.e. the view to world matrix.
    pub world_from_cascade: Mat4,
    /// The orthographic projection for this cascade.
    pub clip_from_cascade: Mat4,
    /// The view-projection matrix for this cascade, converting world space into light clip space.
    /// Importantly, this is derived and stored separately from `view_transform` and `projection` to
    /// ensure shadow stability.
    pub clip_from_world: Mat4,
    /// Size of each shadow map texel in world units.
    pub texel_size: f32,
}

pub fn clear_directional_light_cascades(mut lights: Query<(&DirectionalLight, &mut Cascades)>) {
    for (directional_light, mut cascades) in lights.iter_mut() {
        if !directional_light.shadows_enabled {
            continue;
        }
        cascades.cascades.clear();
    }
}

pub fn build_directional_light_cascades(
    directional_light_shadow_map: Res<DirectionalLightShadowMap>,
    views: Query<(Entity, &GlobalTransform, &Projection, &Camera)>,
    mut lights: Query<(
        &GlobalTransform,
        &DirectionalLight,
        &CascadeShadowConfig,
        &mut Cascades,
    )>,
) {
    let views = views
        .iter()
        .filter_map(|(entity, transform, projection, camera)| {
            if camera.is_active {
                Some((entity, projection, transform.to_matrix()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    for (transform, directional_light, cascades_config, mut cascades) in &mut lights {
        if !directional_light.shadows_enabled {
            continue;
        }

        // It is very important to the numerical and thus visual stability of shadows that
        // light_to_world has orthogonal upper-left 3x3 and zero translation.
        // Even though only the direction (i.e. rotation) of the light matters, we don't constrain
        // users to not change any other aspects of the transform - there's no guarantee
        // `transform.to_matrix()` will give us a matrix with our desired properties.
        // Instead, we directly create a good matrix from just the rotation.
        let world_from_light = Mat4::from_quat(transform.compute_transform().rotation);
        let light_to_world_inverse = world_from_light.inverse();

        for (view_entity, projection, view_to_world) in views.iter().copied() {
            let camera_to_light_view = light_to_world_inverse * view_to_world;
            let view_cascades = cascades_config
                .bounds
                .iter()
                .enumerate()
                .map(|(idx, far_bound)| {
                    // Negate bounds as -z is camera forward direction.
                    let z_near = if idx > 0 {
                        (1.0 - cascades_config.overlap_proportion)
                            * -cascades_config.bounds[idx - 1]
                    } else {
                        -cascades_config.minimum_distance
                    };
                    let z_far = -far_bound;

                    let corners = projection.get_frustum_corners(z_near, z_far);

                    calculate_cascade(
                        corners,
                        directional_light_shadow_map.size as f32,
                        world_from_light,
                        camera_to_light_view,
                    )
                })
                .collect();
            cascades.cascades.insert(view_entity, view_cascades);
        }
    }
}

/// Returns a [`Cascade`] for the frustum defined by `frustum_corners`.
///
/// The corner vertices should be specified in the following order:
/// first the bottom right, top right, top left, bottom left for the near plane, then similar for the far plane.
fn calculate_cascade(
    frustum_corners: [Vec3A; 8],
    cascade_texture_size: f32,
    world_from_light: Mat4,
    light_from_camera: Mat4,
) -> Cascade {
    let mut min = Vec3A::splat(f32::MAX);
    let mut max = Vec3A::splat(f32::MIN);
    for corner_camera_view in frustum_corners {
        let corner_light_view = light_from_camera.transform_point3a(corner_camera_view);
        min = min.min(corner_light_view);
        max = max.max(corner_light_view);
    }

    // NOTE: Use the larger of the frustum slice far plane diagonal and body diagonal lengths as this
    //       will be the maximum possible projection size. Use the ceiling to get an integer which is
    //       very important for floating point stability later. It is also important that these are
    //       calculated using the original camera space corner positions for floating point precision
    //       as even though the lengths using corner_light_view above should be the same, precision can
    //       introduce small but significant differences.
    // NOTE: The size remains the same unless the view frustum or cascade configuration is modified.
    let cascade_diameter = (frustum_corners[0] - frustum_corners[6])
        .length()
        .max((frustum_corners[4] - frustum_corners[6]).length())
        .ceil();

    // NOTE: If we ensure that cascade_texture_size is a power of 2, then as we made cascade_diameter an
    //       integer, cascade_texel_size is then an integer multiple of a power of 2 and can be
    //       exactly represented in a floating point value.
    let cascade_texel_size = cascade_diameter / cascade_texture_size;
    // NOTE: For shadow stability it is very important that the near_plane_center is at integer
    //       multiples of the texel size to be exactly representable in a floating point value.
    let near_plane_center = Vec3A::new(
        (0.5 * (min.x + max.x) / cascade_texel_size).floor() * cascade_texel_size,
        (0.5 * (min.y + max.y) / cascade_texel_size).floor() * cascade_texel_size,
        // NOTE: max.z is the near plane for right-handed y-up
        max.z,
    );

    // It is critical for `world_to_cascade` to be stable. So rather than forming `cascade_to_world`
    // and inverting it, which risks instability due to numerical precision, we directly form
    // `world_to_cascade` as the reference material suggests.
    let light_to_world_transpose = world_from_light.transpose();
    let cascade_from_world = Mat4::from_cols(
        light_to_world_transpose.x_axis,
        light_to_world_transpose.y_axis,
        light_to_world_transpose.z_axis,
        (-near_plane_center).extend(1.0),
    );

    // Right-handed orthographic projection, centered at `near_plane_center`.
    // NOTE: This is different from the reference material, as we use reverse Z.
    let r = (max.z - min.z).recip();
    let clip_from_cascade = Mat4::from_cols(
        Vec4::new(2.0 / cascade_diameter, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 / cascade_diameter, 0.0, 0.0),
        Vec4::new(0.0, 0.0, r, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 1.0),
    );

    let clip_from_world = clip_from_cascade * cascade_from_world;
    Cascade {
        world_from_cascade: cascade_from_world.inverse(),
        clip_from_cascade,
        clip_from_world,
        texel_size: cascade_texel_size,
    }
}
/// Add this component to make a [`Mesh3d`] not cast shadows.
#[derive(Debug, Component, Reflect, Default)]
#[reflect(Component, Default, Debug)]
pub struct NotShadowCaster;
/// Add this component to make a [`Mesh3d`] not receive shadows.
///
/// **Note:** If you're using diffuse transmission, setting [`NotShadowReceiver`] will
/// cause both “regular” shadows as well as diffusely transmitted shadows to be disabled,
/// even when [`TransmittedShadowReceiver`] is being used.
#[derive(Debug, Component, Reflect, Default)]
#[reflect(Component, Default, Debug)]
pub struct NotShadowReceiver;
/// Add this component to make a [`Mesh3d`] using a PBR material with [`diffuse_transmission`](crate::pbr_material::StandardMaterial::diffuse_transmission)`> 0.0`
/// receive shadows on its diffuse transmission lobe. (i.e. its “backside”)
///
/// Not enabled by default, as it requires carefully setting up [`thickness`](crate::pbr_material::StandardMaterial::thickness)
/// (and potentially even baking a thickness texture!) to match the geometry of the mesh, in order to avoid self-shadow artifacts.
///
/// **Note:** Using [`NotShadowReceiver`] overrides this component.
#[derive(Debug, Component, Reflect, Default)]
#[reflect(Component, Default, Debug)]
pub struct TransmittedShadowReceiver;

/// Add this component to a [`Camera3d`](bevy_core_pipeline::core_3d::Camera3d)
/// to control how to anti-alias shadow edges.
///
/// The different modes use different approaches to
/// [Percentage Closer Filtering](https://developer.nvidia.com/gpugems/gpugems/part-ii-lighting-and-shadows/chapter-11-shadow-map-antialiasing).
#[derive(Debug, Component, ExtractComponent, Reflect, Clone, Copy, PartialEq, Eq, Default)]
#[reflect(Component, Default, Debug, PartialEq, Clone)]
pub enum ShadowFilteringMethod {
    /// Hardware 2x2.
    ///
    /// Fast but poor quality.
    Hardware2x2,
    /// Approximates a fixed Gaussian blur, good when TAA isn't in use.
    ///
    /// Good quality, good performance.
    ///
    /// For directional and spot lights, this uses a [method by Ignacio Castaño
    /// for *The Witness*] using 9 samples and smart filtering to achieve the same
    /// as a regular 5x5 filter kernel.
    ///
    /// [method by Ignacio Castaño for *The Witness*]: https://web.archive.org/web/20230210095515/http://the-witness.net/news/2013/09/shadow-mapping-summary-part-1/
    #[default]
    Gaussian,
    /// A randomized filter that varies over time, good when TAA is in use.
    ///
    /// Good quality when used with `TemporalAntiAliasing`
    /// and good performance.
    ///
    /// For directional and spot lights, this uses a [method by Jorge Jimenez for
    /// *Call of Duty: Advanced Warfare*] using 8 samples in spiral pattern,
    /// randomly-rotated by interleaved gradient noise with spatial variation.
    ///
    /// [method by Jorge Jimenez for *Call of Duty: Advanced Warfare*]: https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare/
    Temporal,
}

/// The [`VisibilityClass`] used for all lights (point, directional, and spot).
pub struct LightVisibilityClass;

/// System sets used to run light-related systems.
#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
pub enum SimulationLightSystems {
    AddClusters,
    AssignLightsToClusters,
    /// System order ambiguities between systems in this set are ignored:
    /// each [`build_directional_light_cascades`] system is independent of the others,
    /// and should operate on distinct sets of entities.
    UpdateDirectionalLightCascades,
    UpdateLightFrusta,
    /// System order ambiguities between systems in this set are ignored:
    /// the order of systems within this set is irrelevant, as the various visibility-checking systems
    /// assumes that their operations are irreversible during the frame.
    CheckLightVisibility,
}

pub fn update_directional_light_frusta(
    mut views: Query<
        (
            &Cascades,
            &DirectionalLight,
            &ViewVisibility,
            &mut CascadesFrusta,
        ),
        (
            // Prevents this query from conflicting with camera queries.
            Without<Camera>,
        ),
    >,
) {
    for (cascades, directional_light, visibility, mut frusta) in &mut views {
        // The frustum is used for culling meshes to the light for shadow mapping
        // so if shadow mapping is disabled for this light, then the frustum is
        // not needed.
        if !directional_light.shadows_enabled || !visibility.get() {
            continue;
        }

        frusta.frusta = cascades
            .cascades
            .iter()
            .map(|(view, cascades)| {
                (
                    *view,
                    cascades
                        .iter()
                        .map(|c| Frustum::from_clip_from_world(&c.clip_from_world))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
    }
}

// NOTE: Run this after assign_lights_to_clusters!
pub fn update_point_light_frusta(
    global_lights: Res<GlobalVisibleClusterableObjects>,
    mut views: Query<(Entity, &GlobalTransform, &PointLight, &mut CubemapFrusta)>,
    changed_lights: Query<
        Entity,
        (
            With<PointLight>,
            Or<(Changed<GlobalTransform>, Changed<PointLight>)>,
        ),
    >,
) {
    let view_rotations = CUBE_MAP_FACES
        .iter()
        .map(|CubeMapFace { target, up }| Transform::IDENTITY.looking_at(*target, *up))
        .collect::<Vec<_>>();

    for (entity, transform, point_light, mut cubemap_frusta) in &mut views {
        // If this light hasn't changed, and neither has the set of global_lights,
        // then we can skip this calculation.
        if !global_lights.is_changed() && !changed_lights.contains(entity) {
            continue;
        }

        // The frusta are used for culling meshes to the light for shadow mapping
        // so if shadow mapping is disabled for this light, then the frusta are
        // not needed.
        // Also, if the light is not relevant for any cluster, it will not be in the
        // global lights set and so there is no need to update its frusta.
        if !point_light.shadows_enabled || !global_lights.entities.contains(&entity) {
            continue;
        }

        let clip_from_view = Mat4::perspective_infinite_reverse_rh(
            core::f32::consts::FRAC_PI_2,
            1.0,
            point_light.shadow_map_near_z,
        );

        // ignore scale because we don't want to effectively scale light radius and range
        // by applying those as a view transform to shadow map rendering of objects
        // and ignore rotation because we want the shadow map projections to align with the axes
        let view_translation = Transform::from_translation(transform.translation());
        let view_backward = transform.back();

        for (view_rotation, frustum) in view_rotations.iter().zip(cubemap_frusta.iter_mut()) {
            let world_from_view = view_translation * *view_rotation;
            let clip_from_world = clip_from_view * world_from_view.to_matrix().inverse();

            *frustum = Frustum::from_clip_from_world_custom_far(
                &clip_from_world,
                &transform.translation(),
                &view_backward,
                point_light.range,
            );
        }
    }
}

pub fn update_spot_light_frusta(
    global_lights: Res<GlobalVisibleClusterableObjects>,
    mut views: Query<
        (Entity, &GlobalTransform, &SpotLight, &mut Frustum),
        Or<(Changed<GlobalTransform>, Changed<SpotLight>)>,
    >,
) {
    for (entity, transform, spot_light, mut frustum) in &mut views {
        // The frusta are used for culling meshes to the light for shadow mapping
        // so if shadow mapping is disabled for this light, then the frusta are
        // not needed.
        // Also, if the light is not relevant for any cluster, it will not be in the
        // global lights set and so there is no need to update its frusta.
        if !spot_light.shadows_enabled || !global_lights.entities.contains(&entity) {
            continue;
        }

        // ignore scale because we don't want to effectively scale light radius and range
        // by applying those as a view transform to shadow map rendering of objects
        let view_backward = transform.back();

        let spot_world_from_view = spot_light_world_from_view(transform);
        let spot_clip_from_view =
            spot_light_clip_from_view(spot_light.outer_angle, spot_light.shadow_map_near_z);
        let clip_from_world = spot_clip_from_view * spot_world_from_view.inverse();

        *frustum = Frustum::from_clip_from_world_custom_far(
            &clip_from_world,
            &transform.translation(),
            &view_backward,
            spot_light.range,
        );
    }
}

fn shrink_entities(visible_entities: &mut Vec<Entity>) {
    // Check that visible entities capacity() is no more than two times greater than len()
    let capacity = visible_entities.capacity();
    let reserved = capacity
        .checked_div(visible_entities.len())
        .map_or(0, |reserve| {
            if reserve > 2 {
                capacity / (reserve / 2)
            } else {
                capacity
            }
        });

    visible_entities.shrink_to(reserved);
}

pub fn check_dir_light_mesh_visibility(
    mut commands: Commands,
    mut directional_lights: Query<
        (
            &DirectionalLight,
            &CascadesFrusta,
            &mut CascadesVisibleEntities,
            Option<&RenderLayers>,
            &ViewVisibility,
        ),
        Without<SpotLight>,
    >,
    visible_entity_query: Query<
        (
            Entity,
            &InheritedVisibility,
            Option<&RenderLayers>,
            Option<&Aabb>,
            Option<&GlobalTransform>,
            Has<VisibilityRange>,
            Has<NoFrustumCulling>,
        ),
        (
            Without<NotShadowCaster>,
            Without<DirectionalLight>,
            With<Mesh3d>,
        ),
    >,
    visible_entity_ranges: Option<Res<VisibleEntityRanges>>,
    mut defer_visible_entities_queue: Local<Parallel<Vec<Entity>>>,
    mut view_visible_entities_queue: Local<Parallel<Vec<Vec<Entity>>>>,
) {
    let visible_entity_ranges = visible_entity_ranges.as_deref();

    for (directional_light, frusta, mut visible_entities, maybe_view_mask, light_view_visibility) in
        &mut directional_lights
    {
        let mut views_to_remove = Vec::new();
        for (view, cascade_view_entities) in &mut visible_entities.entities {
            match frusta.frusta.get(view) {
                Some(view_frusta) => {
                    cascade_view_entities.resize(view_frusta.len(), Default::default());
                    cascade_view_entities.iter_mut().for_each(|x| x.clear());
                }
                None => views_to_remove.push(*view),
            };
        }
        for (view, frusta) in &frusta.frusta {
            visible_entities
                .entities
                .entry(*view)
                .or_insert_with(|| vec![VisibleMeshEntities::default(); frusta.len()]);
        }

        for v in views_to_remove {
            visible_entities.entities.remove(&v);
        }

        // NOTE: If shadow mapping is disabled for the light then it must have no visible entities
        if !directional_light.shadows_enabled || !light_view_visibility.get() {
            continue;
        }

        let view_mask = maybe_view_mask.unwrap_or_default();

        for (view, view_frusta) in &frusta.frusta {
            visible_entity_query.par_iter().for_each_init(
                || {
                    let mut entities = view_visible_entities_queue.borrow_local_mut();
                    entities.resize(view_frusta.len(), Vec::default());
                    (defer_visible_entities_queue.borrow_local_mut(), entities)
                },
                |(defer_visible_entities_local_queue, view_visible_entities_local_queue),
                 (
                    entity,
                    inherited_visibility,
                    maybe_entity_mask,
                    maybe_aabb,
                    maybe_transform,
                    has_visibility_range,
                    has_no_frustum_culling,
                )| {
                    if !inherited_visibility.get() {
                        return;
                    }

                    let entity_mask = maybe_entity_mask.unwrap_or_default();
                    if !view_mask.intersects(entity_mask) {
                        return;
                    }

                    // Check visibility ranges.
                    if has_visibility_range
                        && visible_entity_ranges.is_some_and(|visible_entity_ranges| {
                            !visible_entity_ranges.entity_is_in_range_of_view(entity, *view)
                        })
                    {
                        return;
                    }

                    if let (Some(aabb), Some(transform)) = (maybe_aabb, maybe_transform) {
                        let mut visible = false;
                        for (frustum, frustum_visible_entities) in view_frusta
                            .iter()
                            .zip(view_visible_entities_local_queue.iter_mut())
                        {
                            // Disable near-plane culling, as a shadow caster could lie before the near plane.
                            if !has_no_frustum_culling
                                && !frustum.intersects_obb(aabb, &transform.affine(), false, true)
                            {
                                continue;
                            }
                            visible = true;

                            frustum_visible_entities.push(entity);
                        }
                        if visible {
                            defer_visible_entities_local_queue.push(entity);
                        }
                    } else {
                        defer_visible_entities_local_queue.push(entity);
                        for frustum_visible_entities in view_visible_entities_local_queue.iter_mut()
                        {
                            frustum_visible_entities.push(entity);
                        }
                    }
                },
            );
            // collect entities from parallel queue
            for entities in view_visible_entities_queue.iter_mut() {
                visible_entities
                    .entities
                    .get_mut(view)
                    .unwrap()
                    .iter_mut()
                    .zip(entities.iter_mut())
                    .for_each(|(dst, source)| {
                        dst.append(source);
                    });
            }
        }

        for (_, cascade_view_entities) in &mut visible_entities.entities {
            cascade_view_entities
                .iter_mut()
                .map(DerefMut::deref_mut)
                .for_each(shrink_entities);
        }
    }

    // Defer marking view visibility so this system can run in parallel with check_point_light_mesh_visibility
    // TODO: use resource to avoid unnecessary memory alloc
    let mut defer_queue = core::mem::take(defer_visible_entities_queue.deref_mut());
    commands.queue(move |world: &mut World| {
        world.resource_scope::<PreviousVisibleEntities, _>(
            |world, mut previous_visible_entities| {
                let mut query = world.query::<(Entity, &mut ViewVisibility)>();
                for entities in defer_queue.iter_mut() {
                    let mut iter = query.iter_many_mut(world, entities.iter());
                    while let Some((entity, mut view_visibility)) = iter.fetch_next() {
                        if !**view_visibility {
                            view_visibility.set();
                        }

                        // Remove any entities that were discovered to be
                        // visible from the `PreviousVisibleEntities` resource.
                        previous_visible_entities.remove(&entity);
                    }
                }
            },
        );
    });
}

pub fn check_point_light_mesh_visibility(
    visible_point_lights: Query<&VisibleClusterableObjects>,
    mut point_lights: Query<(
        &PointLight,
        &GlobalTransform,
        &CubemapFrusta,
        &mut CubemapVisibleEntities,
        Option<&RenderLayers>,
    )>,
    mut spot_lights: Query<(
        &SpotLight,
        &GlobalTransform,
        &Frustum,
        &mut VisibleMeshEntities,
        Option<&RenderLayers>,
    )>,
    mut visible_entity_query: Query<
        (
            Entity,
            &InheritedVisibility,
            &mut ViewVisibility,
            Option<&RenderLayers>,
            Option<&Aabb>,
            Option<&GlobalTransform>,
            Has<VisibilityRange>,
            Has<NoFrustumCulling>,
        ),
        (
            Without<NotShadowCaster>,
            Without<DirectionalLight>,
            With<Mesh3d>,
        ),
    >,
    visible_entity_ranges: Option<Res<VisibleEntityRanges>>,
    mut previous_visible_entities: ResMut<PreviousVisibleEntities>,
    mut cubemap_visible_entities_queue: Local<Parallel<[Vec<Entity>; 6]>>,
    mut spot_visible_entities_queue: Local<Parallel<Vec<Entity>>>,
    mut checked_lights: Local<EntityHashSet>,
) {
    checked_lights.clear();

    let visible_entity_ranges = visible_entity_ranges.as_deref();
    for visible_lights in &visible_point_lights {
        for light_entity in visible_lights.entities.iter().copied() {
            if !checked_lights.insert(light_entity) {
                continue;
            }

            // Point lights
            if let Ok((
                point_light,
                transform,
                cubemap_frusta,
                mut cubemap_visible_entities,
                maybe_view_mask,
            )) = point_lights.get_mut(light_entity)
            {
                for visible_entities in cubemap_visible_entities.iter_mut() {
                    visible_entities.entities.clear();
                }

                // NOTE: If shadow mapping is disabled for the light then it must have no visible entities
                if !point_light.shadows_enabled {
                    continue;
                }

                let view_mask = maybe_view_mask.unwrap_or_default();
                let light_sphere = Sphere {
                    center: Vec3A::from(transform.translation()),
                    radius: point_light.range,
                };

                visible_entity_query.par_iter_mut().for_each_init(
                    || cubemap_visible_entities_queue.borrow_local_mut(),
                    |cubemap_visible_entities_local_queue,
                     (
                        entity,
                        inherited_visibility,
                        mut view_visibility,
                        maybe_entity_mask,
                        maybe_aabb,
                        maybe_transform,
                        has_visibility_range,
                        has_no_frustum_culling,
                    )| {
                        if !inherited_visibility.get() {
                            return;
                        }
                        let entity_mask = maybe_entity_mask.unwrap_or_default();
                        if !view_mask.intersects(entity_mask) {
                            return;
                        }
                        if has_visibility_range
                            && visible_entity_ranges.is_some_and(|visible_entity_ranges| {
                                !visible_entity_ranges.entity_is_in_range_of_any_view(entity)
                            })
                        {
                            return;
                        }

                        // If we have an aabb and transform, do frustum culling
                        if let (Some(aabb), Some(transform)) = (maybe_aabb, maybe_transform) {
                            let model_to_world = transform.affine();
                            // Do a cheap sphere vs obb test to prune out most meshes outside the sphere of the light
                            if !has_no_frustum_culling
                                && !light_sphere.intersects_obb(aabb, &model_to_world)
                            {
                                return;
                            }

                            for (frustum, visible_entities) in cubemap_frusta
                                .iter()
                                .zip(cubemap_visible_entities_local_queue.iter_mut())
                            {
                                if has_no_frustum_culling
                                    || frustum.intersects_obb(aabb, &model_to_world, true, true)
                                {
                                    if !**view_visibility {
                                        view_visibility.set();
                                    }
                                    visible_entities.push(entity);
                                }
                            }
                        } else {
                            if !**view_visibility {
                                view_visibility.set();
                            }
                            for visible_entities in cubemap_visible_entities_local_queue.iter_mut()
                            {
                                visible_entities.push(entity);
                            }
                        }
                    },
                );

                for entities in cubemap_visible_entities_queue.iter_mut() {
                    for (dst, source) in
                        cubemap_visible_entities.iter_mut().zip(entities.iter_mut())
                    {
                        // Remove any entities that were discovered to be
                        // visible from the `PreviousVisibleEntities` resource.
                        for entity in source.iter() {
                            previous_visible_entities.remove(entity);
                        }

                        dst.entities.append(source);
                    }
                }

                for visible_entities in cubemap_visible_entities.iter_mut() {
                    shrink_entities(visible_entities);
                }
            }

            // Spot lights
            if let Ok((point_light, transform, frustum, mut visible_entities, maybe_view_mask)) =
                spot_lights.get_mut(light_entity)
            {
                visible_entities.clear();

                // NOTE: If shadow mapping is disabled for the light then it must have no visible entities
                if !point_light.shadows_enabled {
                    continue;
                }

                let view_mask = maybe_view_mask.unwrap_or_default();
                let light_sphere = Sphere {
                    center: Vec3A::from(transform.translation()),
                    radius: point_light.range,
                };

                visible_entity_query.par_iter_mut().for_each_init(
                    || spot_visible_entities_queue.borrow_local_mut(),
                    |spot_visible_entities_local_queue,
                     (
                        entity,
                        inherited_visibility,
                        mut view_visibility,
                        maybe_entity_mask,
                        maybe_aabb,
                        maybe_transform,
                        has_visibility_range,
                        has_no_frustum_culling,
                    )| {
                        if !inherited_visibility.get() {
                            return;
                        }

                        let entity_mask = maybe_entity_mask.unwrap_or_default();
                        if !view_mask.intersects(entity_mask) {
                            return;
                        }
                        // Check visibility ranges.
                        if has_visibility_range
                            && visible_entity_ranges.is_some_and(|visible_entity_ranges| {
                                !visible_entity_ranges.entity_is_in_range_of_any_view(entity)
                            })
                        {
                            return;
                        }

                        if let (Some(aabb), Some(transform)) = (maybe_aabb, maybe_transform) {
                            let model_to_world = transform.affine();
                            // Do a cheap sphere vs obb test to prune out most meshes outside the sphere of the light
                            if !has_no_frustum_culling
                                && !light_sphere.intersects_obb(aabb, &model_to_world)
                            {
                                return;
                            }

                            if has_no_frustum_culling
                                || frustum.intersects_obb(aabb, &model_to_world, true, true)
                            {
                                if !**view_visibility {
                                    view_visibility.set();
                                }
                                spot_visible_entities_local_queue.push(entity);
                            }
                        } else {
                            if !**view_visibility {
                                view_visibility.set();
                            }
                            spot_visible_entities_local_queue.push(entity);
                        }
                    },
                );

                for entities in spot_visible_entities_queue.iter_mut() {
                    visible_entities.append(entities);

                    // Remove any entities that were discovered to be visible
                    // from the `PreviousVisibleEntities` resource.
                    for entity in entities {
                        previous_visible_entities.remove(entity);
                    }
                }

                shrink_entities(visible_entities.deref_mut());
            }
        }
    }
}
