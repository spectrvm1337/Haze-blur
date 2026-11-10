//! Direct2D effects for Composition.
//!
//! Composition takes effects as `IGraphicsEffect` objects that describe a Direct2D effect (what Win2D
//! provides in C#, and Win2D does not run in desktop apps). Two are needed, so one small type
//! describes both. Their amounts are named properties, so they can be changed and animated on the
//! brush without building the effect again.
use windows::core::{implement, Interface, GUID, HSTRING};
use windows::Graphics::Effects::{IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl};
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::Direct2D::{CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation};
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING,
    GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT,
};

/// The animated properties, as the brush knows them.
pub(crate) const BLUR_AMOUNT: &str = "blur.BlurAmount";
pub(crate) const SATURATION: &str = "saturation.Saturation";

/// D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.
const OPTIMIZATION_BALANCED: u32 = 1;
/// D2D1_BORDER_MODE_HARD: the edges are not blurred into transparency.
const BORDER_MODE_HARD: u32 = 1;

enum Value {
    Float(f32),
    Uint(u32),
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct Effect {
    name: &'static str,
    clsid: GUID,
    /// In the order of the Direct2D effect's properties.
    values: Vec<Value>,
    /// The name of the property that can be animated, which is always property 0.
    animated: &'static str,
    source: IGraphicsEffectSource,
}

/// Blur by `amount` px (the standard deviation).
pub(crate) fn gaussian_blur(amount: f32, source: IGraphicsEffectSource) -> IGraphicsEffect {
    Effect {
        name: "blur",
        clsid: CLSID_D2D1GaussianBlur,
        values: vec![Value::Float(amount), Value::Uint(OPTIMIZATION_BALANCED), Value::Uint(BORDER_MODE_HARD)],
        animated: "BlurAmount",
        source,
    }
    .into()
}

/// Colour saturation: 0 is grey, 1 leaves the colours alone, above 1 strengthens them.
pub(crate) fn saturation(amount: f32, source: IGraphicsEffectSource) -> IGraphicsEffect {
    Effect { name: "saturation", clsid: CLSID_D2D1Saturation, values: vec![Value::Float(amount)], animated: "Saturation", source }.into()
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
        if unsafe { name.to_string() }.unwrap_or_default() != self.animated {
            return Err(E_INVALIDARG.into());
        }
        unsafe {
            *index = 0;
            *mapping = GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
        }
        Ok(())
    }
    fn GetPropertyCount(&self) -> windows::core::Result<u32> {
        Ok(self.values.len() as u32)
    }
    fn GetProperty(&self, index: u32) -> windows::core::Result<windows::Foundation::IPropertyValue> {
        use windows::Foundation::PropertyValue;
        match self.values.get(index as usize) {
            Some(Value::Float(v)) => PropertyValue::CreateSingle(*v)?.cast(),
            Some(Value::Uint(v)) => PropertyValue::CreateUInt32(*v)?.cast(),
            None => Err(E_INVALIDARG.into()),
        }
    }
    fn GetSource(&self, index: u32) -> windows::core::Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(E_INVALIDARG.into())
        }
    }
    fn GetSourceCount(&self) -> windows::core::Result<u32> {
        Ok(1)
    }
}
