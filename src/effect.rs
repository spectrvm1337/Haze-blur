//! The Gaussian blur as a Composition effect.
//!
//! Composition takes effects as `IGraphicsEffect` objects that describe a Direct2D effect (what Win2D
//! provides in C#, and Win2D does not run in desktop apps). Only the one effect is needed, so it is
//! described here. Its blur amount is a named property, so it can be changed and animated on the
//! brush without building the effect again.
use windows::core::{implement, Interface, HSTRING};
use windows::Graphics::Effects::{IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl};
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING,
    GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT,
};

/// The effect's name inside the brush: the animated property is `"blur.BlurAmount"`.
pub(crate) const EFFECT_NAME: &str = "blur";
/// The name the blur amount goes by.
pub(crate) const AMOUNT: &str = "BlurAmount";

/// CLSID_D2D1GaussianBlur.
const D2D1_GAUSSIAN_BLUR: windows::core::GUID = windows::core::GUID::from_u128(0x1feb6d69_2fe6_4ac9_8c58_1d7f93e7a6a5);
/// D2D1_BORDER_MODE_HARD: the edges are not blurred into transparency.
const BORDER_MODE_HARD: u32 = 1;
/// D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.
const OPTIMIZATION_BALANCED: u32 = 1;

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
pub(crate) struct GaussianBlur {
    pub amount: f32,
    pub source: IGraphicsEffectSource,
}

impl IGraphicsEffect_Impl for GaussianBlur_Impl {
    fn Name(&self) -> windows::core::Result<HSTRING> {
        Ok(HSTRING::from(EFFECT_NAME))
    }
    fn SetName(&self, _name: &HSTRING) -> windows::core::Result<()> {
        Ok(())
    }
}

impl IGraphicsEffectSource_Impl for GaussianBlur_Impl {}

impl IGraphicsEffectD2D1Interop_Impl for GaussianBlur_Impl {
    fn GetEffectId(&self) -> windows::core::Result<windows::core::GUID> {
        Ok(D2D1_GAUSSIAN_BLUR)
    }
    // The blur amount is D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, property 0, set straight.
    fn GetNamedPropertyMapping(
        &self,
        name: &windows::core::PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> windows::core::Result<()> {
        let name = unsafe { name.to_string() }.unwrap_or_default();
        if name == AMOUNT {
            unsafe {
                *index = 0;
                *mapping = GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
            }
            Ok(())
        } else {
            Err(windows::Win32::Foundation::E_INVALIDARG.into())
        }
    }
    fn GetPropertyCount(&self) -> windows::core::Result<u32> {
        Ok(3)
    }
    // In D2D1_GAUSSIANBLUR_PROP order: standard deviation, optimization, border mode.
    fn GetProperty(&self, index: u32) -> windows::core::Result<windows::Foundation::IPropertyValue> {
        use windows::Foundation::PropertyValue;
        match index {
            0 => PropertyValue::CreateSingle(self.amount)?.cast(),
            1 => PropertyValue::CreateUInt32(OPTIMIZATION_BALANCED)?.cast(),
            2 => PropertyValue::CreateUInt32(BORDER_MODE_HARD)?.cast(),
            _ => Err(windows::Win32::Foundation::E_INVALIDARG.into()),
        }
    }
    fn GetSource(&self, index: u32) -> windows::core::Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(windows::Win32::Foundation::E_INVALIDARG.into())
        }
    }
    fn GetSourceCount(&self) -> windows::core::Result<u32> {
        Ok(1)
    }
}
