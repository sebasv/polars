use std::ops::Div;

use arrow::array::{Array, PrimitiveArray};
use arrow::bitmap::Bitmap;
use arrow::compute::utils::combine_validities_and;
use arrow::types::NativeType;
use num_traits::{NumCast, ToPrimitive};

use super::*;
use crate::chunked_array::sum::sum_slice;

fn sum_between_offsets<T, S>(values: &[T], offset: &[i64]) -> Vec<S>
where
    T: NativeType + ToPrimitive,
    S: NumCast + std::iter::Sum,
{
    offset
        .windows(2)
        .map(|w| {
            values
                .get(w[0] as usize..w[1] as usize)
                .map(sum_slice)
                .unwrap_or(S::from(0).unwrap())
        })
        .collect()
}

fn dispatch_sum<T, S>(arr: &dyn Array, offsets: &[i64], validity: Option<&Bitmap>) -> ArrayRef
where
    T: NativeType + ToPrimitive,
    S: NativeType + NumCast + std::iter::Sum,
{
    let values = arr.as_any().downcast_ref::<PrimitiveArray<T>>().unwrap();
    let values = values.values().as_slice();
    Box::new(PrimitiveArray::from_data_default(
        sum_between_offsets::<_, S>(values, offsets).into(),
        validity.cloned(),
    )) as ArrayRef
}

pub(super) fn sum_list_numerical(ca: &ListChunked, inner_type: &DataType) -> Series {
    use DataType::*;
    let chunks = ca
        .downcast_iter()
        .map(|arr| {
            let offsets = arr.offsets().as_slice();
            let values = arr.values().as_ref();

            match inner_type {
                Int8 => dispatch_sum::<i8, i64>(values, offsets, arr.validity()),
                Int16 => dispatch_sum::<i16, i64>(values, offsets, arr.validity()),
                Int32 => dispatch_sum::<i32, i32>(values, offsets, arr.validity()),
                Int64 => dispatch_sum::<i64, i64>(values, offsets, arr.validity()),
                Int128 => dispatch_sum::<i128, i128>(values, offsets, arr.validity()),
                UInt8 => dispatch_sum::<u8, i64>(values, offsets, arr.validity()),
                UInt16 => dispatch_sum::<u16, i64>(values, offsets, arr.validity()),
                UInt32 => dispatch_sum::<u32, u32>(values, offsets, arr.validity()),
                UInt64 => dispatch_sum::<u64, u64>(values, offsets, arr.validity()),
                Float32 => dispatch_sum::<f32, f32>(values, offsets, arr.validity()),
                Float64 => dispatch_sum::<f64, f64>(values, offsets, arr.validity()),
                _ => unimplemented!(),
            }
        })
        .collect::<Vec<_>>();

    Series::try_from((ca.name().clone(), chunks)).unwrap()
}

pub(super) fn sum_with_nulls(ca: &ListChunked, inner_dtype: &DataType) -> PolarsResult<Series> {
    use DataType::*;
    // TODO: add fast path for smaller ints?
    let mut out = match inner_dtype {
        Boolean => {
            let out: IdxCa =
                ca.apply_amortized_generic(|s| s.map(|s| s.as_ref().sum::<IdxSize>().unwrap()));
            out.into_series()
        },
        UInt32 => {
            let out: UInt32Chunked =
                ca.apply_amortized_generic(|s| s.map(|s| s.as_ref().sum::<u32>().unwrap()));
            out.into_series()
        },
        UInt64 => {
            let out: UInt64Chunked =
                ca.apply_amortized_generic(|s| s.map(|s| s.as_ref().sum::<u64>().unwrap()));
            out.into_series()
        },
        Int32 => {
            let out: Int32Chunked =
                ca.apply_amortized_generic(|s| s.map(|s| s.as_ref().sum::<i32>().unwrap()));
            out.into_series()
        },
        Int64 => {
            let out: Int64Chunked =
                ca.apply_amortized_generic(|s| s.map(|s| s.as_ref().sum::<i64>().unwrap()));
            out.into_series()
        },
        Float32 => {
            let out: Float32Chunked =
                ca.apply_amortized_generic(|s| s.map(|s| s.as_ref().sum::<f32>().unwrap()));
            out.into_series()
        },
        Float64 => {
            let out: Float64Chunked =
                ca.apply_amortized_generic(|s| s.map(|s| s.as_ref().sum::<f64>().unwrap()));
            out.into_series()
        },
        // slowest sum_as_series path
        dt => ca
            .try_apply_amortized(|s| {
                s.as_ref()
                    .sum_reduce()
                    .map(|sc| sc.into_series(PlSmallStr::EMPTY))
            })?
            .explode()
            .unwrap()
            .into_series()
            .cast(dt)?,
    };
    out.rename(ca.name().clone());
    Ok(out)
}

fn mean_between_offsets<T, S>(values: &[T], offset: &[i64]) -> PrimitiveArray<S>
where
    T: NativeType + ToPrimitive,
    S: NativeType + NumCast + std::iter::Sum + Div<Output = S>,
{
    offset
        .windows(2)
        .map(|w| {
            values
                .get(w[0] as usize..w[1] as usize)
                .filter(|sl| !sl.is_empty())
                .map(|sl| sum_slice::<_, S>(sl) / NumCast::from(sl.len()).unwrap())
        })
        .collect()
}

fn dispatch_mean<T, S>(arr: &dyn Array, offsets: &[i64], validity: Option<&Bitmap>) -> ArrayRef
where
    T: NativeType + ToPrimitive,
    S: NativeType + NumCast + std::iter::Sum + Div<Output = S>,
{
    let values = arr.as_any().downcast_ref::<PrimitiveArray<T>>().unwrap();
    let values = values.values().as_slice();
    let out = mean_between_offsets::<_, S>(values, offsets);
    let new_validity = combine_validities_and(out.validity(), validity);
    out.with_validity(new_validity).to_boxed()
}

pub(super) fn mean_list_numerical(ca: &ListChunked, inner_type: &DataType) -> Series {
    use DataType::*;
    let chunks = ca
        .downcast_iter()
        .map(|arr| {
            let offsets = arr.offsets().as_slice();
            let values = arr.values().as_ref();

            match inner_type {
                Int8 => dispatch_mean::<i8, f64>(values, offsets, arr.validity()),
                Int16 => dispatch_mean::<i16, f64>(values, offsets, arr.validity()),
                Int32 => dispatch_mean::<i32, f64>(values, offsets, arr.validity()),
                Int64 => dispatch_mean::<i64, f64>(values, offsets, arr.validity()),
                Int128 => dispatch_mean::<i128, f64>(values, offsets, arr.validity()),
                UInt8 => dispatch_mean::<u8, f64>(values, offsets, arr.validity()),
                UInt16 => dispatch_mean::<u16, f64>(values, offsets, arr.validity()),
                UInt32 => dispatch_mean::<u32, f64>(values, offsets, arr.validity()),
                UInt64 => dispatch_mean::<u64, f64>(values, offsets, arr.validity()),
                Float32 => dispatch_mean::<f32, f32>(values, offsets, arr.validity()),
                Float64 => dispatch_mean::<f64, f64>(values, offsets, arr.validity()),
                _ => unimplemented!(),
            }
        })
        .collect::<Vec<_>>();

    Series::try_from((ca.name().clone(), chunks)).unwrap()
}

pub(super) fn mean_with_nulls(ca: &ListChunked) -> Series {
    match ca.inner_dtype() {
        DataType::Float32 => {
            let out: Float32Chunked = ca
                .apply_amortized_generic(|s| s.and_then(|s| s.as_ref().mean().map(|v| v as f32)))
                .with_name(ca.name().clone());
            out.into_series()
        },
        #[cfg(feature = "dtype-datetime")]
        DataType::Date => {
            const MS_IN_DAY: i64 = 86_400_000;
            let out: Int64Chunked = ca
                .apply_amortized_generic(|s| {
                    s.and_then(|s| s.as_ref().mean().map(|v| (v * (MS_IN_DAY as f64)) as i64))
                })
                .with_name(ca.name().clone());
            out.into_datetime(TimeUnit::Milliseconds, None)
                .into_series()
        },
        dt if dt.is_temporal() => {
            let out: Int64Chunked = ca
                .apply_amortized_generic(|s| s.and_then(|s| s.as_ref().mean().map(|v| v as i64)))
                .with_name(ca.name().clone());
            out.cast(dt).unwrap()
        },
        _ => {
            let out: Float64Chunked = ca
                .apply_amortized_generic(|s| s.and_then(|s| s.as_ref().mean()))
                .with_name(ca.name().clone());
            out.into_series()
        },
    }
}
