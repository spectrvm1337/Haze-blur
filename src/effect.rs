//! Direct2D effects for Composition.
//!
//! Composition takes effects as `IGraphicsEffect` objects that describe a Direct2D effect (what Win2D
//! provides in C#, and Win2D does not run in desktop apps). The blur is a chain of at most four:
//!
//! ```text
//! backdrop -> saturation -> Gaussian blur -> composite <- flood (the tint)
//! ```
//!
//! A link that would do nothing is left out ([`Chain`]), so a plain blur costs one pass and the
//! backdrop alone costs none. Every amount is a named property, driven by an expression on the brush,
//! so changing it never builds the effect again.
use windows::core::{implement, Interface, GUID, HSTRING};
use windows::Graphics::Effects::{IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectSource_Impl, IGraphicsEffect_Impl};
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::Direct2D::{CLSID_D2D1Composite, CLSID_D2D1Flood, CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation};
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING,
    GRAPHICS_EFFECT_PROPERTY_MAPPING_COLOR_TO_VECTOR4, GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT,
};
use windows::UI::Composition::CompositionEffectSourceParameter;

/// The source parameter the backdrop brush is plugged into.
pub(crate) const SOURCE: &str = "backdrop";

/// Which links of the chain a brush has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chain(u8);

impl Chain {
    pub(crate) const NONE: Chain = Chain(0);
    pub(crate) const BLUR: Chain = Chain(1);
    pub(crate) const SATURATION: Chain = Chain(2);
    pub(crate) const TINT: Chain = Chain(4);
    /// How many different chains there are.
    pub(crate) const COUNT: usize = 8;

    pub(crate) fn has(self, link: Chain) -> bool {
        self.0 & link.0 != 0
    }
    pub(crate) fn with(self, link: Chain, on: bool) -> Chain {
        if on {
            Chain(self.0 | link.0)
        } else {
            Chain(self.0 & !link.0)
        }
    }
    pub(crate) fn union(self, other: Chain) -> Chain {
        Chain(self.0 | other.0)
    }
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
    #[cfg(test)]
    pub(crate) fn all() -> impl Iterator<Item = Chain> {
        (0..Self::COUNT as u8).map(Chain)
    }

    /// The effect for this chain and the properties it binds, as `(property, expression)` over a
    /// property set named `s`. `None` for the empty chain: the backdrop brush is used as it is.
    #[allow(clippy::type_complexity)]
    pub(crate) fn effect(self) -> windows::core::Result<Option<(IGraphicsEffect, Vec<(&'static str, &'static str)>)>> {
        if self == Chain::NONE {
            return Ok(None);
        }
        let mut source: IGraphicsEffectSource = CompositionEffectSourceParameter::Create(&HSTRING::from(SOURCE))?.cast()?;
        let mut bound = Vec::new();
        if self.has(Chain::SATURATION) {
            source = saturation(source).cast()?;
            bound.push(("saturation.Saturation", "s.Saturation"));
        }
        if self.has(Chain::BLUR) {
            source = gaussian_blur(source).cast()?;
            bound.push(("blur.BlurAmount", "s.Strength * s.Scale"));
        }
        if self.has(Chain::TINT) {
            source = tint_over(source)?.cast()?;
            bound.push(("tint.Color", "s.TintColor"));
        }
        Ok(Some((source.cast()?, bound)))
    }
}

/// D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.
const OPTIMIZATION_BALANCED: u32 = 1;
/// D2D1_BORDER_MODE_HARD: the edges are not blurred into transparency.
const BORDER_MODE_HARD: u32 = 1;
/// D2D1_COMPOSITE_MODE_SOURCE_OVER.
const SOURCE_OVER: u32 = 0;

enum Value {
    Float(f32),
    Uint(u32),
    Vector4([f32; 4]),
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct Effect {
    name: &'static str,
    clsid: GUID,
    /// In the order of the Direct2D effect's properties.
    values: Vec<Value>,
    /// The property that can be animated: its name, its index, and how Composition maps it.
    animated: Option<(&'static str, u32, GRAPHICS_EFFECT_PROPERTY_MAPPING)>,
    sources: Vec<IGraphicsEffectSource>,
}

/// Blur by `BlurAmount` px (the standard deviation).
fn gaussian_blur(source: IGraphicsEffectSource) -> IGraphicsEffect {
    Effect {
        name: "blur",
        clsid: CLSID_D2D1GaussianBlur,
        values: vec![Value::Float(0.0), Value::Uint(OPTIMIZATION_BALANCED), Value::Uint(BORDER_MODE_HARD)],
        animated: Some(("BlurAmount", 0, GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT)),
        sources: vec![source],
    }
    .into()
}

/// Colour saturation: 0 is grey, 1 leaves the colours alone, above 1 strengthens them.
fn saturation(source: IGraphicsEffectSource) -> IGraphicsEffect {
    Effect {
        name: "saturation",
        clsid: CLSID_D2D1Saturation,
        values: vec![Value::Float(1.0)],
        animated: Some(("Saturation", 0, GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT)),
        sources: vec![source],
    }
    .into()
}

/// `source` with a flat colour over it; the colour's alpha says how much of it.
fn tint_over(source: IGraphicsEffectSource) -> windows::core::Result<IGraphicsEffect> {
    let flood: IGraphicsEffect = Effect {
        name: "tint",
        clsid: CLSID_D2D1Flood,
        values: vec![Value::Vector4([0.0; 4])],
        animated: Some(("Color", 0, GRAPHICS_EFFECT_PROPERTY_MAPPING_COLOR_TO_VECTOR4)),
        sources: Vec::new(),
    }
    .into();
    Ok(Effect {
        name: "composite",
        clsid: CLSID_D2D1Composite,
        values: vec![Value::Uint(SOURCE_OVER)],
        animated: None,
        // The destination first, then what goes over it.
        sources: vec![source, flood.cast()?],
    }
    .into())
}

impl IGraphicsEffect_Impl for Effect_Impl {
    fn Name(&self) -> windows::core::Result<HSTRING> {
        Ok(HSTRING::from(self.name))
    }
    fn SetName(&self, _name: &HSTRING) -> windows::core::Result<()> {
        Ok(())
    }
}

impl IGraphicsEffectSource_Impl for Effect_Impl {}

impl IGraphicsEffectD2D1Interop_Impl for Effect_Impl {
    fn GetEffectId(&self) -> windows::core::Result<GUID> {
        Ok(self.clsid)
    }
    fn GetNamedPropertyMapping(
        &self,
        name: &windows::core::PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> windows::core::Result<()> {
        let asked = unsafe { name.to_string() }.unwrap_or_default();
        match self.animated {
            Some((known, i, how)) if known == asked => {
                unsafe {
                    *index = i;
                    *mapping = how;
                }
                Ok(())
            }
            _ => Err(E_INVALIDARG.into()),
        }
    }
    fn GetPropertyCount(&self) -> windows::core::Result<u32> {
        Ok(self.values.len() as u32)
    }
    fn GetProperty(&self, index: u32) -> windows::core::Result<windows::Foundation::IPropertyValue> {
        use windows::Foundation::PropertyValue;
        match self.values.get(index as usize) {
            Some(Value::Float(v)) => PropertyValue::CreateSingle(*v)?.cast(),
            Some(Value::Uint(v)) => PropertyValue::CreateUInt32(*v)?.cast(),
            Some(Value::Vector4(v)) => PropertyValue::CreateSingleArray(v)?.cast(),
            None => Err(E_INVALIDARG.into()),
        }
    }
    fn GetSource(&self, index: u32) -> windows::core::Result<IGraphicsEffectSource> {
        self.sources.get(index as usize).cloned().ok_or_else(|| E_INVALIDARG.into())
    }
    fn GetSourceCount(&self) -> windows::core::Result<u32> {
        Ok(self.sources.len() as u32)
    }
}
