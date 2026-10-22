import { useCallback, useEffect, useRef, useState } from 'react';

/**
 * Reports when an element has been scrolled into view, and stops watching once
 * it has. Everything on this page enters once; nothing animates on the way out,
 * because content that moves when you scroll back up is harder to read, not
 * more alive.
 *
 * Only the bottom edge is inset, so entering is held back until an element is
 * properly on screen. Insetting the top as well leaves a dead band under the
 * masthead where anything already in view on load never reveals at all.
 */
export function useInView<T extends HTMLElement>(rootMargin = '0px 0px -10% 0px') {
  const ref = useRef<T | null>(null);
  const [inView, setInView] = useState(false);

  useEffect(() => {
    const node = ref.current;
    if (!node || inView) return;

    // Anything already on screen when it mounts is answered from geometry on a
    // timer rather than from the observer. An observer only reports during a
    // rendering update, so a tab that is throttled or not yet painted can hold
    // its first callback indefinitely, and a page whose content is hidden until
    // that callback arrives would stay blank. The timer also lets the first
    // frame paint at rest, so what is already in view fades in rather than
    // appearing finished.
    const timer = window.setTimeout(() => {
      if (node.getBoundingClientRect().top < window.innerHeight) setInView(true);
    }, 60);

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setInView(true);
          observer.disconnect();
        }
      },
      { rootMargin },
    );

    observer.observe(node);
    return () => {
      window.clearTimeout(timer);
      observer.disconnect();
    };
  }, [inView, rootMargin]);

  return { ref, inView };
}

/** Whether the visitor has asked for less motion. */
export function useReducedMotion() {
  // Rendered on the server during the prerender step, where there is no
  // matchMedia. Assuming motion is wanted matches what the client resolves for
  // almost everyone, and the effect corrects it for the rest before anything
  // has moved.
  const [reduced, setReduced] = useState(
    () =>
      typeof window !== 'undefined' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches,
  );

  useEffect(() => {
    const query = window.matchMedia('(prefers-reduced-motion: reduce)');
    const onChange = () => setReduced(query.matches);
    query.addEventListener('change', onChange);
    return () => query.removeEventListener('change', onChange);
  }, []);

  return reduced;
}

/** Copies text, and reports for a moment that it did. */
export function useCopy(value: string) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const copy = useCallback(() => {
    void (async () => {
      try {
        await navigator.clipboard.writeText(value);
      } catch {
        // A browser that refuses the clipboard says nothing; the text is on
        // screen and selectable either way.
        return;
      }
      setCopied(true);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setCopied(false), 1600);
    })();
  }, [value]);

  return { copied, copy };
}

/** Whether the page has been scrolled past a threshold. */
export function useScrolled(threshold = 8) {
  const [scrolled, setScrolled] = useState(false);

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > threshold);
    onScroll();
    window.addEventListener('scroll', onScroll, { passive: true });
    return () => window.removeEventListener('scroll', onScroll);
  }, [threshold]);

  return scrolled;
}
