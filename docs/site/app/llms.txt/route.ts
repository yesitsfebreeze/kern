import { llmsTxt, basePath } from '@/lib/llm-txt.mjs';

export const revalidate = false;

export function GET(request: Request) {
  if (process.env.NODE_ENV === 'development') {
    return new Response(llmsTxt(new URL(request.url).origin + basePath));
  }
  return new Response(llmsTxt());
}
