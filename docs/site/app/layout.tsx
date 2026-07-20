import { RootProvider } from 'fumadocs-ui/provider/next';
import type { ReactNode } from 'react';
import type { Metadata } from 'next';
import SearchDialog from '@/components/search';
import './global.css';

export const metadata: Metadata = {
  title: {
    template: '%s — Kern',
    default: 'Kern',
  },
  description: 'A self-learning memory substrate for AI agents.',
};

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <RootProvider search={{ SearchDialog }}>{children}</RootProvider>
      </body>
    </html>
  );
}
