import { Footer } from './components/Footer.tsx';
import { Nav } from './components/Nav.tsx';
import { Ci } from './components/sections/Ci.tsx';
import { Features } from './components/sections/Features.tsx';
import { Files } from './components/sections/Files.tsx';
import { Hero } from './components/sections/Hero.tsx';
import { Start } from './components/sections/Start.tsx';
import { VersionBadge } from './components/ui/VersionBadge.tsx';

export function App() {
  return (
    <>
      <a
        href="#top"
        className="focus:bg-signal focus:text-signal-ink sr-only focus:not-sr-only focus:fixed focus:top-3 focus:left-3 focus:z-[60] focus:rounded-full focus:px-4 focus:py-2"
      >
        Skip to content
      </a>
      <Nav />
      <main>
        <Hero />
        <Features />
        <Files />
        <Ci />
        <Start />
      </main>
      <Footer />
      <VersionBadge />
    </>
  );
}
