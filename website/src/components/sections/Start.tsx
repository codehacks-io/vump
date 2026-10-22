import { links, VUMP_VERSION } from '../../content/links.ts';
import { initRun } from '../../content/runs.ts';
import { Button } from '../ui/Button.tsx';
import { Code } from '../ui/Code.tsx';
import { ArrowIcon } from '../ui/icons.tsx';
import { Reveal } from '../ui/Reveal.tsx';
import { Terminal } from '../ui/Terminal.tsx';

const SOURCE = `git clone https://github.com/codehacks-io/vump
cd vump && cargo install --path .`;

export function Start() {
  return (
    <section id="start" className="border-line relative overflow-hidden border-t py-20 md:py-28">
      <div className="grid-backdrop pointer-events-none absolute inset-0 -z-10" aria-hidden />
      <div
        className="pointer-events-none absolute -bottom-72 left-1/2 -z-10 h-[36rem] w-[60rem] -translate-x-1/2 rounded-full bg-[radial-gradient(closest-side,var(--glow),transparent)]"
        aria-hidden
      />

      <div className="mx-auto w-full max-w-[78rem] px-6 md:px-10">
        <Reveal>
          <p className="label text-signal">Quick start</p>
          <h2 className="mt-5 max-w-[18ch] text-3xl leading-[1.08] font-medium tracking-[-0.03em] md:text-[2.6rem]">
            Three steps, once.
          </h2>
        </Reveal>

        <div className="mt-12 grid gap-8 lg:grid-cols-3 lg:gap-6">
          <Reveal>
            <div className="flex h-full min-w-0 flex-col">
              <p className="label">
                <span className="text-signal">01</span> &nbsp;Install it
              </p>
              <p className="text-muted mt-3 mb-5 text-[15px] leading-relaxed">
                Grab a binary for your platform, or build it from source.
              </p>
              <Button
                href={links.latestRelease}
                variant="outline"
                external
                className="mb-4 self-start"
              >
                Download v{VUMP_VERSION}
                <ArrowIcon className="opacity-60" />
              </Button>
              <Code code={SOURCE} lang="bash" />
            </div>
          </Reveal>

          <Reveal delay={70}>
            <div className="flex h-full min-w-0 flex-col">
              <p className="label">
                <span className="text-signal">02</span> &nbsp;Point it at your files
              </p>
              <p className="text-muted mt-3 mb-5 text-[15px] leading-relaxed">
                <code className="text-ink font-mono text-[13px]">vump init</code> finds the files
                that record your version and writes them into a config you can edit.
              </p>
              <Terminal lines={initRun} title="~/your-repo" animate />
            </div>
          </Reveal>

          <Reveal delay={140}>
            <div className="flex h-full min-w-0 flex-col">
              <p className="label">
                <span className="text-signal">03</span> &nbsp;Bump
              </p>
              <p className="text-muted mt-3 mb-5 text-[15px] leading-relaxed">
                Run <code className="text-ink font-mono text-[13px]">vump</code> and pick a bump, or
                name one and skip the questions.
              </p>
              <div className="border-line bg-raised/40 flex flex-1 flex-col justify-center gap-3 rounded-xl border p-6 font-mono text-[13px]">
                <p>
                  <span className="text-signal">$</span> vump
                  <span className="text-faint"> — guided</span>
                </p>
                <p>
                  <span className="text-signal">$</span> vump patch
                  <span className="text-faint"> — no questions</span>
                </p>
                <p>
                  <span className="text-signal">$</span> vump patch --through push
                </p>
              </div>
            </div>
          </Reveal>
        </div>

        <Reveal delay={100}>
          <div className="border-line mt-14 flex flex-col gap-4 border-t pt-8 sm:flex-row sm:items-center sm:justify-between">
            <p className="text-muted text-[15px]">
              Flags, configuration and everything else live in the manual.
            </p>
            <Button href={links.readme} variant="outline" external className="self-start">
              Read the docs
              <ArrowIcon className="opacity-60" />
            </Button>
          </div>
        </Reveal>
      </div>
    </section>
  );
}
