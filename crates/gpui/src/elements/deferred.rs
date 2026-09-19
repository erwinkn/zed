use crate::{
    AnyElement, App, Bounds, Element, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Window,
};

/// Builds a `Deferred` element, which delays the layout and paint of its child.
pub fn deferred(child: impl IntoElement) -> Deferred {
    Deferred {
        child: Some(child.into_any_element()),
        priority: 0,
    }
}

/// An element which delays the painting of its child until after all of
/// its ancestors, while keeping its layout as part of the current element tree.
pub struct Deferred {
    child: Option<AnyElement>,
    priority: usize,
}

impl Deferred {
    /// Sets the `priority` value of the `deferred` element, which
    /// determines the drawing order relative to other deferred elements,
    /// with higher values being drawn on top.
    pub fn with_priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
}

impl Element for Deferred {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<crate::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let layout_id = self.child.as_mut().unwrap().request_layout(window, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let child = self.child.take().unwrap();
        let element_offset = window.element_offset();
        window.defer_draw(child, element_offset, self.priority, None)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}

impl IntoElement for Deferred {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Deferred {
    /// Sets a priority for the element. A higher priority conceptually means painting the element
    /// on top of deferred draws with a lower priority (i.e. closer to the viewer).
    pub fn priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Context, Entity, StyleRefinement, TestAppContext, Window, anchored, deferred, div, point,
        prelude::*, px, size,
    };

    /// A stand-in for a dock panel hosting a popover (deferred draw) whose
    /// content opens another popover (a deferred draw created while
    /// prepainting the first one's content).
    struct PanelView;

    impl Render for PanelView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().key_context("Panel").size_full().child(
                deferred(
                    anchored().position(point(px(10.), px(10.))).child(
                        div().key_context("Popover").w(px(200.)).h(px(200.)).child(
                            deferred(
                                anchored().position(point(px(30.), px(30.))).child(
                                    div()
                                        .key_context("NestedMenu")
                                        .debug_selector(|| "NESTED_MENU".into())
                                        .w(px(50.))
                                        .h(px(50.)),
                                ),
                            )
                            .with_priority(2),
                        ),
                    ),
                )
                .with_priority(1),
            )
        }
    }

    struct RootView {
        panel: Entity<PanelView>,
    }

    impl Render for RootView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().key_context("Root").size_full().child(
                self.panel
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            )
        }
    }

    /// Regression test for a crash with nested deferred draws (e.g. a popover
    /// menu inside a popover hosted by a cached dock panel). Prepaint indices
    /// recorded during the deferred draw rounds must index the same
    /// `deferred_draws` vector that `reuse_prepaint` slices on the next frame;
    /// previously they were measured against a transient per-round vector, so
    /// reusing the panel's subtree grafted the wrong deferred draws and
    /// panicked in the dispatch tree.
    #[gpui::test]
    fn test_nested_deferred_draws_with_reused_views(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(800.), px(600.)), |_, cx| {
            let panel = cx.new(|_| PanelView);
            RootView { panel }
        });
        cx.run_until_parked();

        let menu_bounds = window
            .update(cx, |_, window, _| {
                window
                    .rendered_frame
                    .debug_bounds
                    .get("NESTED_MENU")
                    .copied()
            })
            .unwrap()
            .expect("NESTED_MENU debug bounds not found");
        assert_eq!(menu_bounds.size, size(px(50.), px(50.)));

        // Re-render only the root view; the panel is cached, so its subtree -
        // including both deferred draw records - is reused from the previous
        // frame.
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();

        // Reuse the subtree a second time, exercising ranges that were
        // themselves recorded during a reused frame.
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();

        // Re-render the panel itself again to prove the popovers still draw.
        window
            .update(cx, |root, _, cx| {
                root.panel.update(cx, |_, cx| cx.notify());
            })
            .unwrap();
        cx.run_until_parked();

        window
            .update(cx, |_, window, _| {
                assert_eq!(window.rendered_frame.deferred_draws.len(), 2);
                assert!(
                    window
                        .rendered_frame
                        .debug_bounds
                        .contains_key("NESTED_MENU")
                );
            })
            .unwrap();
    }

    struct ContextView;
    impl Render for ContextView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                crate::canvas(
                    |_, window, cx| {
                        assert!(window.element_context::<usize>().is_none());
                        window.with_element_context(std::rc::Rc::new("label"), |window| {
                            window.with_element_context(std::rc::Rc::new(11usize), |window| {
                                let child = crate::canvas(
                                    |_, window, cx| {
                                        assert_eq!(window.element_context::<usize>(), Some(&11));
                                        window.with_element_context(
                                            std::rc::Rc::new(22usize),
                                            |window| {
                                                defer_context_probe(window, cx, 22, 2);
                                            },
                                        );
                                        assert_eq!(window.element_context::<usize>(), Some(&11));
                                    },
                                    |_, _, window, cx| {
                                        assert_eq!(window.element_context::<usize>(), Some(&11));
                                        assert_eq!(
                                            window.element_context::<&str>(),
                                            Some(&"label")
                                        );
                                        cx.global_mut::<ContextLog>().0.push(11);
                                    },
                                )
                                .size_full();
                                let mut child = child.into_any_element();
                                child.layout_as_root(
                                    size(
                                        crate::AvailableSpace::Definite(px(20.)),
                                        crate::AvailableSpace::Definite(px(20.)),
                                    ),
                                    window,
                                    cx,
                                );
                                window.defer_draw(child, point(px(0.), px(0.)), 0, None);
                            });
                            assert!(window.element_context::<usize>().is_none());
                            assert_eq!(window.element_context::<&str>(), Some(&"label"));
                        });
                        assert!(window.element_context::<&str>().is_none());
                        let mut sibling = crate::canvas(
                            |_, window, _| {
                                assert!(window.element_context::<usize>().is_none());
                                assert!(window.element_context::<&str>().is_none());
                            },
                            |_, _, window, cx| {
                                assert!(window.element_context::<usize>().is_none());
                                assert!(window.element_context::<&str>().is_none());
                                cx.global_mut::<ContextLog>().0.push(0);
                            },
                        )
                        .size_full()
                        .into_any_element();
                        sibling.layout_as_root(
                            size(
                                crate::AvailableSpace::Definite(px(20.)),
                                crate::AvailableSpace::Definite(px(20.)),
                            ),
                            window,
                            cx,
                        );
                        window.defer_draw(sibling, point(px(30.), px(0.)), 1, None);
                    },
                    |_, _, window, _| assert!(window.element_context::<usize>().is_none()),
                )
                .size_full(),
            )
        }
    }

    fn defer_context_probe(
        window: &mut Window,
        cx: &mut crate::App,
        expected: usize,
        priority: usize,
    ) {
        let mut child = crate::canvas(
            move |_, window, _| {
                assert_eq!(window.element_context::<usize>(), Some(&expected));
                assert_eq!(window.element_context::<&str>(), Some(&"label"));
            },
            move |_, _, window, cx| {
                assert_eq!(window.element_context::<usize>(), Some(&expected));
                assert_eq!(window.element_context::<&str>(), Some(&"label"));
                cx.global_mut::<ContextLog>().0.push(expected);
            },
        )
        .size_full()
        .into_any_element();
        child.layout_as_root(
            size(
                crate::AvailableSpace::Definite(px(20.)),
                crate::AvailableSpace::Definite(px(20.)),
            ),
            window,
            cx,
        );
        window.defer_draw(child, point(px(0.), px(0.)), priority, None);
    }

    #[derive(Default)]
    struct ContextLog(Vec<usize>);
    impl crate::Global for ContextLog {}

    #[gpui::test]
    fn test_element_context_survives_nested_deferred_draws(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(ContextLog::default()));
        let handle = cx.add_window(|_, _| ContextView);
        for _ in 0..2 {
            cx.update_window(handle.into(), |_, window, cx| {
                cx.global_mut::<ContextLog>().0.clear();
                window.draw(cx).clear(cx);
                assert_eq!(cx.global::<ContextLog>().0, vec![11, 0, 22]);
                assert!(window.element_context::<usize>().is_none());
                assert!(window.element_context::<&str>().is_none());
            })
            .unwrap();
        }
    }

    struct CachedContextRoot(Entity<ContextView>);
    impl Render for CachedContextRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0
                .clone()
                .cached(StyleRefinement::default().size_full())
        }
    }

    #[gpui::test]
    fn test_cached_deferred_context_does_not_leak_or_repeat_paint(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(ContextLog::default()));
        let handle = cx.add_window(|_, cx| CachedContextRoot(cx.new(|_| ContextView)));
        cx.run_until_parked();
        for _ in 0..2 {
            cx.update(|cx| cx.global_mut::<ContextLog>().0.clear());
            handle.update(cx, |_, _, cx| cx.notify()).unwrap();
            cx.run_until_parked();
            handle
                .update(cx, |_, window, cx| {
                    assert!(cx.global::<ContextLog>().0.is_empty());
                    assert_eq!(window.rendered_frame.deferred_draws.len(), 3);
                    assert!(window.element_context::<usize>().is_none());
                })
                .unwrap();
        }
        handle
            .update(cx, |root, _, cx| {
                root.0.update(cx, |_, cx| cx.notify());
            })
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(cx.global::<ContextLog>().0, vec![11, 0, 22]));
    }

    fn completion_probe(index: usize) -> impl IntoElement {
        div()
            .w(px(10.))
            .h(px(10.))
            .debug_selector(move || format!("complete-{index}"))
            .on_painted(move |_, window, cx| {
                cx.global_mut::<ContextLog>().0.push(index);
                window.on_draw_complete(move |window, cx| {
                    assert!(
                        window
                            .rendered_frame
                            .debug_bounds
                            .contains_key(&format!("complete-{index}"))
                    );
                    assert!(window.element_context::<usize>().is_none());
                    cx.global_mut::<ContextLog>().0.push(index + 10);
                });
            })
    }
    struct CompletionView;
    impl Render for CompletionView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(completion_probe(1)).child(
                deferred(
                    div()
                        .child(completion_probe(2))
                        .child(deferred(completion_probe(3)).with_priority(2)),
                )
                .with_priority(1),
            )
        }
    }
    #[gpui::test]
    fn test_draw_completion_follows_all_deferred_paint(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(ContextLog::default()));
        let handle = cx.add_window(|_, _| CompletionView);
        for _ in 0..2 {
            cx.update_window(handle.into(), |_, window, cx| {
                cx.global_mut::<ContextLog>().0.clear();
                window.draw(cx).clear(cx);
                assert_eq!(cx.global::<ContextLog>().0, vec![1, 2, 3, 11, 12, 13]);
                assert!(!window.invalidator.is_dirty());
            })
            .unwrap();
        }
    }

    struct CachedCompletionRoot(Entity<CompletionView>);
    impl Render for CachedCompletionRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0
                .clone()
                .cached(StyleRefinement::default().size_full())
        }
    }
    #[gpui::test]
    fn test_draw_completion_is_not_repeated_by_cached_paint(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(ContextLog::default()));
        let handle = cx.add_window(|_, cx| CachedCompletionRoot(cx.new(|_| CompletionView)));
        cx.run_until_parked();
        for _ in 0..2 {
            cx.update(|cx| cx.global_mut::<ContextLog>().0.clear());
            handle.update(cx, |_, _, cx| cx.notify()).unwrap();
            cx.run_until_parked();
            cx.update(|cx| assert!(cx.global::<ContextLog>().0.is_empty()));
        }
    }
}
