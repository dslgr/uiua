use std::{
    cmp::Ordering,
    fmt::{self, Debug, Display},
    iter::repeat,
    ops::Deref,
    rc::Rc,
    slice::{Chunks, ChunksMut},
};

use crate::{function::Function, primitive::Primitive, Byte, Uiua, UiuaResult};

#[derive(Clone)]
pub struct Array<T> {
    pub(crate) shape: Vec<usize>,
    pub(crate) data: Vec<T>,
    pub(crate) fill: bool,
}

impl<T: ArrayValue> Default for Array<T> {
    fn default() -> Self {
        Self {
            shape: vec![0],
            data: Vec::new(),
            fill: T::DEFAULT_FILL,
        }
    }
}
impl<T: ArrayValue> fmt::Debug for Array<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl<T: ArrayValue> fmt::Display for Array<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.rank() {
            0 => write!(f, "{}", self.data[0]),
            1 => {
                let (start, end) = T::format_delims();
                write!(f, "{}", start)?;
                for (i, x) in self.data.iter().enumerate() {
                    if i > 0 {
                        write!(f, "{}", T::format_sep())?;
                    }
                    write!(f, "{}", x)?;
                }
                write!(f, "{}", end)
            }
            _ => {
                write!(f, "[")?;
                for (i, dim) in self.shape().iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{dim}")?;
                }
                write!(f, ",")?;
                for val in &self.data {
                    write!(f, " {}", val)?;
                }
                write!(f, "]")
            }
        }
    }
}

#[track_caller]
#[inline(always)]
fn validate_shape<T>(shape: &[usize], data: &[T]) {
    debug_assert_eq!(
        shape.iter().product::<usize>(),
        data.len(),
        "shape {shape:?} does not match data length {}",
        data.len()
    );
}

impl<T: ArrayValue> Array<T> {
    #[track_caller]
    pub fn new(shape: Vec<usize>, data: Vec<T>) -> Self {
        validate_shape(&shape, &data);
        Self {
            shape,
            data,
            fill: T::DEFAULT_FILL,
        }
    }
    #[track_caller]
    #[inline(always)]
    pub(crate) fn validate_shape(&self) {
        validate_shape(&self.shape, &self.data);
    }
    pub fn unit(data: T) -> Self {
        Self::new(Vec::new(), vec![data])
    }
    pub fn row_count(&self) -> usize {
        self.shape.first().copied().unwrap_or(1)
    }
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        if self.rank() == 1 {
            self.data.iter().take_while(|x| !x.is_fill_value()).count()
        } else {
            self.row_count()
        }
    }
    pub fn flat_len(&self) -> usize {
        self.data.len()
    }
    pub fn row_len(&self) -> usize {
        self.shape.iter().skip(1).product()
    }
    pub fn rank(&self) -> usize {
        self.shape.len()
    }
    pub fn rows(
        &self,
    ) -> impl ExactSizeIterator<Item = Row<T>> + DoubleEndedIterator<Item = Row<T>> {
        (0..self.row_count()).map(|row| Row { array: self, row })
    }
    pub fn rows_mut(&mut self) -> ChunksMut<T> {
        let row_len = self.row_len();
        self.data.chunks_mut(row_len)
    }
    pub fn row(&self, row: usize) -> &[T] {
        let row_len = self.row_len();
        &self.data[row * row_len..(row + 1) * row_len]
    }
    pub fn reset_fill(&mut self) {
        self.fill = T::DEFAULT_FILL;
    }
    pub fn convert<U>(self) -> Array<U>
    where
        T: Into<U>,
    {
        Array {
            shape: self.shape,
            data: self.data.into_iter().map(Into::into).collect(),
            fill: self.fill,
        }
    }
    pub fn into_rows(self) -> impl Iterator<Item = Self> {
        let row_len = self.row_len();
        let mut row_shape = self.shape.clone();
        let row_count = if row_shape.is_empty() {
            1
        } else {
            row_shape.remove(0)
        };
        let mut data = self.data.into_iter();
        (0..row_count)
            .map(move |_| Array::new(row_shape.clone(), data.by_ref().take(row_len).collect()))
    }
    pub fn into_rows_rev(mut self) -> impl Iterator<Item = Self> {
        let row_len = self.row_len();
        let mut row_shape = self.shape.clone();
        let row_count = if row_shape.is_empty() {
            1
        } else {
            row_shape.remove(0)
        };
        (0..row_count).map(move |_| {
            let end = self.data.len() - row_len;
            Array::new(row_shape.clone(), self.data.drain(end..).collect())
        })
    }
    pub fn val_eq<U: Into<T> + Clone>(&self, other: &Array<U>) -> bool {
        self.shape == other.shape
            && self.data.len() == other.data.len()
            && self
                .data
                .iter()
                .zip(&other.data)
                .all(|(a, b)| T::eq(a, &b.clone().into()))
    }
    pub fn val_cmp<U: Into<T> + Clone>(&self, other: &Array<U>) -> Ordering {
        self.data
            .iter()
            .zip(&other.data)
            .map(|(a, b)| a.cmp(&b.clone().into()))
            .find(|o| o != &Ordering::Equal)
            .unwrap_or_else(|| self.data.len().cmp(&other.data.len()))
    }
    pub fn empty_row(&self) -> Self {
        if self.rank() == 0 {
            return self.clone();
        }
        Array::new(self.shape[1..].to_vec(), Vec::new())
    }
    /// Remove fill elements from the end of the array
    pub fn truncate(&mut self) {
        if !self.fill || self.rank() == 0 {
            return;
        }
        let mut new_len = self.row_count();
        for (i, row) in self.rows().enumerate().rev() {
            if row.iter().all(|x| x.is_fill_value()) {
                new_len = i;
            } else {
                break;
            }
        }
        self.data.truncate(new_len * self.row_len());
        self.shape[0] = new_len;
    }
    #[track_caller]
    pub fn from_row_arrays(
        values: impl IntoIterator<Item = Self>,
        fill: bool,
        env: &Uiua,
    ) -> UiuaResult<Self> {
        let mut row_values = values.into_iter();
        let Some(mut value) = row_values.next() else {
            return Ok(Self::default());
        };
        let mut count = 1;
        for mut row in row_values {
            row.fill |= fill;
            count += 1;
            value = if count == 2 {
                value.couple(row, env)?
            } else {
                value.join_impl(row, false, env)?
            };
        }
        if count == 1 {
            value.shape.insert(0, 1);
        }
        Ok(value)
    }
}

impl<T: ArrayValue> PartialEq for Array<T> {
    fn eq(&self, other: &Self) -> bool {
        if !(self.shape == other.shape && self.data.len() == other.data.len()) {
            return false;
        }
        let a = self
            .data
            .iter()
            .skip_while(|x| x.is_fill_value())
            .take_while(|x| !x.is_fill_value());
        let b = other
            .data
            .iter()
            .skip_while(|x| x.is_fill_value())
            .take_while(|x| !x.is_fill_value());
        a.zip(b).all(|(a, b)| a.cmp(b) == Ordering::Equal)
    }
}

impl<T: ArrayValue> Eq for Array<T> {}

impl<T: ArrayValue> PartialOrd for Array<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.val_cmp(other))
    }
}

impl<T: ArrayValue> Ord for Array<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.data
            .iter()
            .zip(&other.data)
            .map(|(a, b)| a.cmp(b))
            .find(|o| o != &Ordering::Equal)
            .unwrap_or_else(|| self.data.len().cmp(&other.data.len()))
    }
}

impl<T: ArrayValue> From<T> for Array<T> {
    fn from(data: T) -> Self {
        Self::unit(data)
    }
}

impl<T: ArrayValue> From<(Vec<usize>, Vec<T>)> for Array<T> {
    fn from((shape, data): (Vec<usize>, Vec<T>)) -> Self {
        Self::new(shape, data)
    }
}

impl<T: ArrayValue> From<Vec<T>> for Array<T> {
    fn from(data: Vec<T>) -> Self {
        Self::new(vec![data.len()], data)
    }
}

impl<T: ArrayValue> FromIterator<T> for Array<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::from(iter.into_iter().collect::<Vec<T>>())
    }
}

impl From<String> for Array<char> {
    fn from(s: String) -> Self {
        Self::new(vec![s.len()], s.chars().collect())
    }
}

impl FromIterator<String> for Array<char> {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        let mut lines: Vec<String> = iter.into_iter().collect();
        let max_len = lines.iter().map(|s| s.chars().count()).max().unwrap_or(0);
        let mut data = Vec::with_capacity(max_len * lines.len());
        let shape = vec![lines.len(), max_len];
        for line in lines.drain(..) {
            data.extend(line.chars());
            data.extend(repeat('\x00').take(max_len - line.chars().count()));
        }
        Array::new(shape, data)
    }
}

pub struct Row<'a, T> {
    array: &'a Array<T>,
    row: usize,
}

impl<'a, T> Clone for Row<'a, T> {
    fn clone(&self) -> Self {
        Row {
            array: self.array,
            row: self.row,
        }
    }
}

impl<'a, T> Copy for Row<'a, T> {}

impl<'a, T: ArrayValue> AsRef<[T]> for Row<'a, T> {
    fn as_ref(&self) -> &[T] {
        self.array.row(self.row)
    }
}

impl<'a, T: ArrayValue> Deref for Row<'a, T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        self.array.row(self.row)
    }
}

impl<'a, T: ArrayValue> PartialEq for Row<'a, T> {
    fn eq(&self, other: &Self) -> bool {
        self.iter().zip(&**other).all(|(a, b)| a.eq(b))
    }
}

impl<'a, T: ArrayValue> Eq for Row<'a, T> {}

impl<'a, T: ArrayValue> PartialOrd for Row<'a, T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a, T: ArrayValue> Ord for Row<'a, T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.iter()
            .zip(&**other)
            .map(|(a, b)| a.cmp(b))
            .find(|o| o != &Ordering::Equal)
            .unwrap_or_else(|| self.array.data.len().cmp(&other.array.data.len()))
    }
}

impl<'a, T: ArrayValue> Arrayish for Row<'a, T> {
    type Value = T;
    fn shape(&self) -> &[usize] {
        &self.array.shape[1..]
    }
    fn data(&self) -> &[T] {
        self
    }
}

pub trait ArrayValue: Clone + Debug + Display {
    const NAME: &'static str;
    const DEFAULT_FILL: bool = false;
    fn cmp(&self, other: &Self) -> Ordering;
    fn fill_value() -> Self;
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
    fn format_delims() -> (&'static str, &'static str) {
        ("[", "]")
    }
    fn format_sep() -> &'static str {
        " "
    }
    fn is_fill_value(&self) -> bool {
        false
    }
}

impl ArrayValue for f64 {
    const NAME: &'static str = "number";
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other)
            .unwrap_or_else(|| self.is_nan().cmp(&other.is_nan()))
    }
    fn fill_value() -> Self {
        f64::NAN
    }
    fn is_fill_value(&self) -> bool {
        self.is_nan()
    }
}

impl ArrayValue for Byte {
    const NAME: &'static str = "byte";
    fn cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(self, other)
    }
    fn fill_value() -> Self {
        Byte::Fill
    }
    fn is_fill_value(&self) -> bool {
        *self == Byte::Fill
    }
}

impl ArrayValue for char {
    const NAME: &'static str = "character";
    const DEFAULT_FILL: bool = true;
    fn cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(self, other)
    }
    fn format_delims() -> (&'static str, &'static str) {
        ("", "")
    }
    fn format_sep() -> &'static str {
        ""
    }
    fn fill_value() -> Self {
        '\x00'
    }
    fn is_fill_value(&self) -> bool {
        *self == '\x00'
    }
}

impl ArrayValue for Rc<Function> {
    const NAME: &'static str = "function";
    fn cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(self, other)
    }
    fn fill_value() -> Self {
        Rc::new(Primitive::Noop.into())
    }
    fn is_fill_value(&self) -> bool {
        self.as_primitive() == Some(Primitive::Noop)
    }
}

#[allow(clippy::len_without_is_empty)]
pub trait Arrayish {
    type Value: ArrayValue;
    fn shape(&self) -> &[usize];
    fn data(&self) -> &[Self::Value];
    fn rank(&self) -> usize {
        self.shape().len()
    }
    fn flat_len(&self) -> usize {
        self.data().len()
    }
    fn row_len(&self) -> usize {
        self.shape().iter().skip(1).product()
    }
    fn rows(&self) -> Chunks<Self::Value> {
        self.data().chunks(self.row_len())
    }
    fn shape_prefixes_match(&self, other: &impl Arrayish) -> bool {
        self.shape().iter().zip(other.shape()).all(|(a, b)| a == b)
    }
}

impl<'a, T> Arrayish for &'a T
where
    T: Arrayish,
{
    type Value = T::Value;
    fn shape(&self) -> &[usize] {
        T::shape(self)
    }
    fn data(&self) -> &[Self::Value] {
        T::data(self)
    }
}

impl<T: ArrayValue> Arrayish for Array<T> {
    type Value = T;
    fn shape(&self) -> &[usize] {
        &self.shape
    }
    fn data(&self) -> &[Self::Value] {
        &self.data
    }
}

impl<T: ArrayValue> Arrayish for (&[usize], &[T]) {
    type Value = T;
    fn shape(&self) -> &[usize] {
        self.0
    }
    fn data(&self) -> &[Self::Value] {
        self.1
    }
}

impl<T: ArrayValue> Arrayish for (&[usize], &mut [T]) {
    type Value = T;
    fn shape(&self) -> &[usize] {
        self.0
    }
    fn data(&self) -> &[Self::Value] {
        self.1
    }
}
