import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FilesPage } from "./FilesPage";

const filesFixture = {
  items: [
    {
      id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      original_name: "report.pdf",
      content_type: "application/pdf",
      size_bytes: 2048,
      width: null,
      height: null,
      created_at: new Date(Date.now() - 86_400_000).toISOString(),
    },
    {
      id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
      original_name: "cover.png",
      content_type: "image/png",
      size_bytes: 5120,
      width: 512,
      height: 512,
      created_at: new Date(Date.now() - 3_600_000).toISOString(),
    },
  ],
};

const uploadMutate = vi.hoisted(() => vi.fn());

vi.mock("../api/hooks", () => ({
  useMyFiles: () => ({
    data: filesFixture,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useUploadFile: () => ({ mutate: uploadMutate, isPending: false, error: null }),
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderPage() {
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/me/files"]}>
        <FilesPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("FilesPage", () => {
  beforeEach(() => {
    uploadMutate.mockReset();
  });

  it("lists uploaded files with size, dimensions and date", () => {
    renderPage();
    expect(screen.getByRole("heading", { name: "My files" })).toBeInTheDocument();
    expect(screen.getByText("report.pdf")).toBeInTheDocument();
    expect(screen.getByText("cover.png")).toBeInTheDocument();
    expect(screen.getByText("2.0 KB")).toBeInTheDocument();
    expect(screen.getByText(/512×512/)).toBeInTheDocument();
  });

  it("uploads a chosen file", async () => {
    const user = userEvent.setup();
    renderPage();
    const file = new File(["hello"], "notes.txt", { type: "text/plain" });
    await user.upload(screen.getByLabelText("Choose a file to upload"), file);
    await user.click(screen.getByRole("button", { name: "Upload" }));
    expect(uploadMutate).toHaveBeenCalledWith({ file });
  });
});
