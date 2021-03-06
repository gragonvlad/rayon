use super::plumbing::*;
use super::*;

use std::fmt::{self, Debug};

/// `Update` is an iterator that mutates the elements of an
/// underlying iterator before they are yielded.
///
/// This struct is created by the [`update()`] method on [`ParallelIterator`]
///
/// [`update()`]: trait.ParallelIterator.html#method.update
/// [`ParallelIterator`]: trait.ParallelIterator.html
#[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
#[derive(Clone)]
pub struct Update<I: ParallelIterator, F> {
    base: I,
    update_op: F,
}

impl<I: ParallelIterator + Debug, F> Debug for Update<I, F> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Update").field("base", &self.base).finish()
    }
}

/// Create a new `Update` iterator.
///
/// NB: a free fn because it is NOT part of the end-user API.
pub fn new<I, F>(base: I, update_op: F) -> Update<I, F>
where
    I: ParallelIterator,
{
    Update {
        base: base,
        update_op: update_op,
    }
}

impl<I, F> ParallelIterator for Update<I, F>
where
    I: ParallelIterator,
    F: Fn(&mut I::Item) + Send + Sync,
{
    type Item = I::Item;

    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        let consumer1 = UpdateConsumer::new(consumer, &self.update_op);
        self.base.drive_unindexed(consumer1)
    }

    fn opt_len(&self) -> Option<usize> {
        self.base.opt_len()
    }
}

impl<I, F> IndexedParallelIterator for Update<I, F>
where
    I: IndexedParallelIterator,
    F: Fn(&mut I::Item) + Send + Sync,
{
    fn drive<C>(self, consumer: C) -> C::Result
    where
        C: Consumer<Self::Item>,
    {
        let consumer1 = UpdateConsumer::new(consumer, &self.update_op);
        self.base.drive(consumer1)
    }

    fn len(&self) -> usize {
        self.base.len()
    }

    fn with_producer<CB>(self, callback: CB) -> CB::Output
    where
        CB: ProducerCallback<Self::Item>,
    {
        return self.base.with_producer(Callback {
            callback: callback,
            update_op: self.update_op,
        });

        struct Callback<CB, F> {
            callback: CB,
            update_op: F,
        }

        impl<T, F, CB> ProducerCallback<T> for Callback<CB, F>
        where
            CB: ProducerCallback<T>,
            F: Fn(&mut T) + Send + Sync,
        {
            type Output = CB::Output;

            fn callback<P>(self, base: P) -> CB::Output
            where
                P: Producer<Item = T>,
            {
                let producer = UpdateProducer {
                    base: base,
                    update_op: &self.update_op,
                };
                self.callback.callback(producer)
            }
        }
    }
}

/// ////////////////////////////////////////////////////////////////////////

struct UpdateProducer<'f, P, F: 'f> {
    base: P,
    update_op: &'f F,
}

impl<'f, P, F> Producer for UpdateProducer<'f, P, F>
where
    P: Producer,
    F: Fn(&mut P::Item) + Send + Sync,
{
    type Item = P::Item;
    type IntoIter = UpdateSeq<P::IntoIter, &'f F>;

    fn into_iter(self) -> Self::IntoIter {
        UpdateSeq {
            base: self.base.into_iter(),
            update_op: self.update_op,
        }
    }

    fn min_len(&self) -> usize {
        self.base.min_len()
    }
    fn max_len(&self) -> usize {
        self.base.max_len()
    }

    fn split_at(self, index: usize) -> (Self, Self) {
        let (left, right) = self.base.split_at(index);
        (
            UpdateProducer {
                base: left,
                update_op: self.update_op,
            },
            UpdateProducer {
                base: right,
                update_op: self.update_op,
            },
        )
    }

    fn fold_with<G>(self, folder: G) -> G
    where
        G: Folder<Self::Item>,
    {
        let folder1 = UpdateFolder {
            base: folder,
            update_op: self.update_op,
        };
        self.base.fold_with(folder1).base
    }
}

/// ////////////////////////////////////////////////////////////////////////
/// Consumer implementation

struct UpdateConsumer<'f, C, F: 'f> {
    base: C,
    update_op: &'f F,
}

impl<'f, C, F> UpdateConsumer<'f, C, F> {
    fn new(base: C, update_op: &'f F) -> Self {
        UpdateConsumer {
            base: base,
            update_op: update_op,
        }
    }
}

impl<'f, T, C, F> Consumer<T> for UpdateConsumer<'f, C, F>
where
    C: Consumer<T>,
    F: Fn(&mut T) + Send + Sync,
{
    type Folder = UpdateFolder<'f, C::Folder, F>;
    type Reducer = C::Reducer;
    type Result = C::Result;

    fn split_at(self, index: usize) -> (Self, Self, Self::Reducer) {
        let (left, right, reducer) = self.base.split_at(index);
        (
            UpdateConsumer::new(left, self.update_op),
            UpdateConsumer::new(right, self.update_op),
            reducer,
        )
    }

    fn into_folder(self) -> Self::Folder {
        UpdateFolder {
            base: self.base.into_folder(),
            update_op: self.update_op,
        }
    }

    fn full(&self) -> bool {
        self.base.full()
    }
}

impl<'f, T, C, F> UnindexedConsumer<T> for UpdateConsumer<'f, C, F>
where
    C: UnindexedConsumer<T>,
    F: Fn(&mut T) + Send + Sync,
{
    fn split_off_left(&self) -> Self {
        UpdateConsumer::new(self.base.split_off_left(), &self.update_op)
    }

    fn to_reducer(&self) -> Self::Reducer {
        self.base.to_reducer()
    }
}

struct UpdateFolder<'f, C, F: 'f> {
    base: C,
    update_op: &'f F,
}

impl<'f, T, C, F> Folder<T> for UpdateFolder<'f, C, F>
where
    C: Folder<T>,
    F: Fn(&mut T),
{
    type Result = C::Result;

    fn consume(self, mut item: T) -> Self {
        (self.update_op)(&mut item);

        UpdateFolder {
            base: self.base.consume(item),
            update_op: self.update_op,
        }
    }

    fn complete(self) -> C::Result {
        self.base.complete()
    }

    fn full(&self) -> bool {
        self.base.full()
    }
}

/// Standard Update adaptor, based on `itertools::adaptors::Update`
#[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
#[derive(Debug, Clone)]
struct UpdateSeq<I, F> {
    base: I,
    update_op: F,
}

impl<I, F> Iterator for UpdateSeq<I, F>
where
    I: Iterator,
    F: FnMut(&mut I::Item),
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(mut v) = self.base.next() {
            (self.update_op)(&mut v);
            Some(v)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.base.size_hint()
    }

    fn fold<Acc, G>(self, init: Acc, mut g: G) -> Acc
    where
        G: FnMut(Acc, Self::Item) -> Acc,
    {
        let mut f = self.update_op;
        self.base.fold(init, move |acc, mut v| {
            f(&mut v);
            g(acc, v)
        })
    }

    // if possible, re-use inner iterator specializations in collect
    fn collect<C>(self) -> C
    where
        C: ::std::iter::FromIterator<Self::Item>,
    {
        let mut f = self.update_op;
        self.base
            .map(move |mut v| {
                f(&mut v);
                v
            }).collect()
    }
}

impl<I, F> ExactSizeIterator for UpdateSeq<I, F>
where
    I: ExactSizeIterator,
    F: FnMut(&mut I::Item),
{}

impl<I, F> DoubleEndedIterator for UpdateSeq<I, F>
where
    I: DoubleEndedIterator,
    F: FnMut(&mut I::Item),
{
    fn next_back(&mut self) -> Option<Self::Item> {
        if let Some(mut v) = self.base.next_back() {
            (self.update_op)(&mut v);
            Some(v)
        } else {
            None
        }
    }
}
