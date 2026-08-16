import { useState, useEffect } from "react";
import { cn } from "@/lib/utils";

interface SplashScreenProps {
  onFinish?: () => void;
  duration?: number;
  forceShow?: boolean;
}

export function SplashScreen({ onFinish, duration = 3000, forceShow = false }: SplashScreenProps) {
  const [visible, setVisible] = useState(true);
  const [fading, setFading] = useState(false);

  useEffect(() => {
    const fadeTimer = setTimeout(() => {
      setFading(true);
    }, duration);

    const finishTimer = setTimeout(() => {
      setVisible(false);
      onFinish?.();
    }, duration + 450);

    return () => {
      clearTimeout(fadeTimer);
      clearTimeout(finishTimer);
    };
  }, [duration, onFinish, forceShow]);

  if (!visible && !forceShow) return null;

  return (
    <div
      className={cn(
        "fixed inset-0 z-[99999] flex flex-col items-center justify-center bg-[#09090b] text-[#f4f4f5] select-none transition-opacity duration-450 ease-out",
        fading && !forceShow ? "opacity-0 pointer-events-none" : "opacity-100"
      )}
    >
      {/* Background Ambient Glow */}
      <div className="absolute w-72 h-72 rounded-full bg-emerald-500/12 blur-[90px] pointer-events-none animate-pulse duration-1000" />

      <div className="relative flex flex-col items-center gap-7">
        {/* Animated SVG Logo */}
        <div className="relative flex items-center justify-center">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            viewBox="150 280 1650 450"
            className="h-16 w-auto drop-shadow-[0_0_35px_rgba(16,185,129,0.25)]"
          >
            <g transform="translate(18.4, 102.4) scale(0.8)">
              {/* White Geometric 'M' Base with draw-in animation */}
              <path
                d="M 232 742 L 232 382 L 382 532 L 512 402"
                stroke="#F4F4F5"
                strokeWidth="72"
                strokeLinecap="round"
                strokeLinejoin="round"
                fill="none"
                pathLength="100"
                className="mm-path-m"
              />

              {/* Green Checkmark with energetic draw-in & glow */}
              <path
                d="M 512 402 L 612 502 L 792 282"
                stroke="#10B981"
                strokeWidth="72"
                strokeLinecap="round"
                strokeLinejoin="round"
                fill="none"
                pathLength="100"
                className="mm-path-check"
              />
            </g>

            {/* "MergeMark" Text with soft fade-in & tracking */}
            <text
              x="716"
              y="512"
              dominantBaseline="central"
              fontFamily="system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif"
              fontSize="160"
              fontWeight="bold"
              fill="#FFFFFF"
              className="mm-text"
            >
              MergeMark
            </text>
          </svg>
        </div>

        {/* Minimalist Glowing Progress Track */}
        <div className="w-36 h-[2.5px] bg-white/10 rounded-full overflow-hidden relative transform translate-z-0">
          <div
            className="absolute top-0 left-0 h-full w-[35%] rounded-full bg-gradient-to-r from-emerald-400 to-blue-500 will-change-transform"
            style={{
              animation: "splash-slide 1.3s cubic-bezier(0.4, 0, 0.2, 1) infinite alternate",
            }}
          />
        </div>
      </div>

      <style>{`
        .mm-path-m {
          stroke-dasharray: 100;
          stroke-dashoffset: 100;
          animation: mmDrawM 0.7s cubic-bezier(0.4, 0, 0.2, 1) forwards;
        }
        .mm-path-check {
          stroke-dasharray: 100;
          stroke-dashoffset: 100;
          animation: mmDrawCheck 0.6s cubic-bezier(0.16, 1, 0.3, 1) 0.35s forwards;
        }
        .mm-text {
          opacity: 0;
          animation: mmFadeText 0.6s ease-out 0.25s forwards;
        }
        @keyframes mmDrawM {
          0% { stroke-dashoffset: 100; opacity: 0.2; }
          100% { stroke-dashoffset: 0; opacity: 1; }
        }
        @keyframes mmDrawCheck {
          0% { stroke-dashoffset: 100; filter: drop-shadow(0 0 0px #10B981); }
          50% { filter: drop-shadow(0 0 20px #10B981); }
          100% { stroke-dashoffset: 0; filter: drop-shadow(0 0 10px #10B981); }
        }
        @keyframes mmFadeText {
          0% { opacity: 0; transform: translateX(-8px); }
          100% { opacity: 1; transform: translateX(0); }
        }
        @keyframes splash-slide {
          0% { transform: translateX(-10%); }
          100% { transform: translateX(200%); }
        }
      `}</style>
    </div>
  );
}
