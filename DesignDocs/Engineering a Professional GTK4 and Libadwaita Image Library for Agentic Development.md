# **Engineering a Professional GTK4 and Libadwaita Image Library for Agentic Development**

The modern landscape of Linux application development has been fundamentally reshaped by the convergence of the GTK4 toolkit and the Libadwaita library. This evolution marks a transition from purely functional user interfaces to sophisticated, adaptive experiences that prioritize performance, visual consistency, and security through sandboxing. For an image library application that serves both as a high-speed preview tool and an AI-powered enhancement suite, the architectural choices made at the inception of the project determine its ultimate viability in a crowded ecosystem. The transition from legacy utilities such as Eye of GNOME (EOG) to modern alternatives like Loupe highlights a broader industry trend: the shift toward memory-safe languages like Rust, GPU-accelerated rendering pipelines, and the use of sandboxed image decoding via frameworks like Glycin to mitigate security risks associated with untrusted media files.1

## **The Agentic Development Paradigm and Language Selection**

In designing an application specifically for execution by agentic coding tools such as Claude Code or Codex, the choice of programming language transcends simple performance metrics. The language must serve as a communication layer that provides structured, actionable feedback to the AI agent. Rust, coupled with the gtk4-rs and libadwaita-rs bindings, emerges as the most compatible foundation for this paradigm. The inherent design of the Rust compiler, which enforces strict memory safety and ownership rules at compile-time, acts as an automated peer reviewer for the agent.2 When an agent generates code that violates a lifetime constraint or introduces a potential data race, the compiler does not merely report an error; it provides a detailed diagnostic that explains the violation and often suggests a corrective path. This creates a remarkably tight feedback loop that allows the agent to iteratively refine the implementation without human intervention, ensuring that the final binary is free from entire classes of runtime bugs common in C++ or Python-based GTK applications.3  
The deterministic nature of the Rust compiler contrasts sharply with the indeterminism of agentic logic. By defining the application's types and traits first, the developer sets clear boundaries within which the agent can implement logic. This approach leverages the strengths of the AI—rapid generation of boilerplate and logical implementation—while the compiler enforces the structural integrity of the codebase. Furthermore, the use of cargo clippy and cargo fmt as part of the agent's execution cycle ensures that the generated code adheres to idiomatic standards, which is critical for long-term maintainability.3

| Evaluation Criterion | Rust (GTK-RS) | Python (PyGObject) | C (GObject/GTK4) |
| :---- | :---- | :---- | :---- |
| **Agent Feedback Loop** | Exceptional (Compiler diagnostics) 3 | Moderate (Pylint/Type hints) 3 | Poor (Memory leaks, SEGFAULTs) 2 |
| **Safety and Security** | High (Memory safety, Sandboxing) 1 | Moderate (Managed memory) 4 | Low (Manual memory management) 2 |
| **Performance Profile** | High (Native machine code) 5 | Low (Interpreted overhead) 6 | High (Native machine code) |
| **Concurrency Model** | Safe (Send/Sync traits) 5 | Limited (Global Interpreter Lock) 7 | Manual (Pthreads/GThread) 8 |
| **Deployment Ease** | Simple (Self-contained binary) 5 | Complex (Env management) 5 | Moderate (Dynamic linking) 9 |

While Python is often lauded for its rapid prototyping capabilities in the AI research domain, its reliance on runtime interpretation and its difficulties with true parallelism make it less suitable for a performance-intensive image library that must handle 4K resolution rendering and concurrent file scanning.6 Rust provides C++-level performance with the safety of a high-level language, which is essential when the application is designed to be built by an agent that may not have the nuanced understanding of manual memory allocation required for robust C development.2

## **Adaptive Triple-Pane UI Architecture**

The core of the application's user experience is its three-pane layout, which must balance the needs of a file explorer, a navigation filmstrip, and a high-fidelity image preview. Adhering to the GNOME Human Interface Guidelines (HIG) requires this layout to be adaptive, responding seamlessly to changes in window size or device form factor.11 The implementation utilizes a nested split-view structure to manage these three regions.

### **Structural Hierarchy of Split Views**

The primary container is the AdwNavigationSplitView, which manages the high-level relationship between the application's utility panes and its main content area. Within the content area of this split view, an AdwOverlaySplitView is nested to create the distinction between the vertical thumbnail strip and the primary image viewer. This hierarchy allows for independent control over how each pane collapses under constrained width conditions.11  
The AdwNavigationSplitView is responsible for the leftmost sidebar, which serves as the file explorer. This sidebar contains a folder tree and collection filters (such as "Recent" or "Tags"). In an agentic implementation, the sidebar should be defined as an AdwNavigationPage, ensuring that when the application window is narrowed, the sidebar can collapse into a separate view accessible via a back button or a view switcher.11 The use of AdwBreakpoint is critical here; a breakpoint should be set at approximately 1200px to hide the explorer sidebar by default, transitioning the UI from a "management" mode to a "browsing" mode.11  
The AdwOverlaySplitView handles the relationship between the vertical thumbnail filmstrip and the image preview. Unlike the primary sidebar, the filmstrip is often needed even when window space is reduced, albeit in a more compact form. By setting the sidebar-position to the start and using the .navigation-sidebar style class, the thumbnail strip maintains the visual weight expected in a standard GNOME application.15 For widths below 800px, the application should programmatically toggle the collapsed property of the AdwOverlaySplitView, overlaying the thumbnail strip only when requested by the user, thereby maximizing the screen real estate for the image preview itself.11

### **Dynamic Layout Transitions**

To achieve a truly professional polish, the application must implement "progressive disclosure," a HIG pattern that involves showing controls only when they are relevant to the user's current task.17 This is particularly relevant for the three-pane layout:

| Layout State | Desktop Mode (\>1200px) | Tablet Mode (800-1200px) | Mobile Mode (\<800px) |
| :---- | :---- | :---- | :---- |
| **Explorer Sidebar** | Permanently visible side-by-side.13 | Hidden; accessible via hamburger menu.11 | Collapsed into root navigation page.11 |
| **Thumbnail Strip** | Visible side-by-side with preview.15 | Visible or auto-hiding overlay.15 | Hidden; available as a bottom sheet.14 |
| **Header Bar** | Unified across all three panes.13 | Dynamic title based on current page.18 | Centered title with back button.13 |

The use of AdwToolbarView as the content child for each pane ensures header bars and bottom bars are consistently styled and integrated with the application's theme. The top-bar style should be set to raised for the primary viewer to provide a clear boundary between the image content and the window controls.11

## **Interaction Design and Standard Shortcuts**

Adherence to the GNOME HIG necessitates a rich set of keyboard and mouse interactions that users expect from a modern platform application.19 These interactions must be defined via GtkShortcutController and GtkGesture subclasses to ensure they are discoverable and consistent.1

### **Keyboard and Mouse Input Mappings**

The application implementation utilizes standard shortcuts to minimize cognitive load for existing GNOME users:

* **Zooming:** Ctrl \+ Mousewheel Up/Down for smooth zooming centered on the cursor.2 Additionally, \+/- keys or Ctrl \+ 0 to reset zoom to "Best Fit" must be supported.2  
* **Navigation:** Alt \+ Left/Right to cycle through images in the current folder.2 Page Up/Down or Home/End should jump through the filmstrip.  
* **Pan & Interaction:** Mouse Drag to pan zoomed images.2 Double Click to toggle between "Fit" and "1:1" zoom levels.1  
* **Global Actions:** Ctrl \+ S to save current modifications, Ctrl \+ , for settings, and Alt \+ Enter to toggle the metadata overlay.2

## **High-Performance Image Lists and Filmstrip Recycling**

A primary failure point for many image library applications is the performance degradation that occurs as the number of managed files grows. Modern image libraries must remain responsive when handling thousands of assets. The filmstrip thumbnail viewer in this application must utilize the GtkListView widget, which replaces the legacy GtkTreeView and GtkListBox for large data sets.20

### **Leveraging GListModel and Widget Recycling**

The GtkListView does not render all items in its model simultaneously. Instead, it employs a recycling mechanism that creates a small pool of GtkListItem widgets, which are then reused to display different data as the user scrolls. This reduces the memory footprint from $O(N)$ to $O(1)$ relative to the number of items in the library.20  
The implementation requires a custom GListModel—typically a GListStore in Rust—to hold ImageEntry objects. Each ImageEntry is a GObject that stores metadata such as the file path, a cached thumbnail texture, and image dimensions.21 The interaction between the model and the view is governed by a GtkSignalListItemFactory, which manages the binding of data to widgets through a four-stage signal lifecycle:

1. **Setup:** The factory emits this signal to create the widget hierarchy for a single row. For the thumbnail strip, this involves creating a GtkBox containing a GtkPicture and a GtkLabel for the filename.20  
2. **Bind:** This signal is emitted when a specific ImageEntry from the model needs to be displayed. The application retrieves the thumbnail texture and filename from the entry and applies them to the widgets created in the setup phase.20  
3. **Unbind:** When a row scrolls out of view, this signal allows the application to release references to the image data, ensuring that only the textures for visible items remain in GPU memory.20  
4. **Teardown:** This final stage destroys the widget hierarchy when the list view itself is closed or the pool of recycled widgets is reduced.20

## **AI Super-Resolution Integration and Upscale UX**

The defining feature of this application is its integration of state-of-the-art AI upscaling directly into the workflow. By utilizing Real-ESRGAN-ncnn-vulkan and waifu2x-ncnn-vulkan, the application provides professional-grade enhancement that leverages the user's GPU without requiring the installation of complex Python environments or CUDA drivers.25

### **Intelligent Progress Feedback**

During the upscaling operation, which may take several seconds, the application must provide immediate visual feedback. A GtkProgressBar is implemented as an overlay or "shaded bar" directly under the primary image preview. Using the .osd (On-Screen Display) CSS class ensures the progress bar remains visible against any background while maintaining a sleek, non-intrusive appearance. The progress bar updates in real-time by parsing the standard error stream of the NCNN process.28

### **Split-View Comparison Interface**

Once the upscale is completed, the preview transitions from a single image to an interactive **Before/After Split Preview**. This is achieved using a custom widget implementation within a GtkOverlay:

* **The Divider:** A grabbable vertical bar allows the user to slide across the image.  
* **The Visualization:** The "Before" image is rendered in the background, while the "After" image is rendered on top. The top image's visibility is clipped based on the position of the divider handle, allowing the user to inspect AI-restored details side-by-side with the original pixels.  
* **State Management:** The user can "Commit" the upscale, which saves it to the upscaled folder, or "Discard" it to return to the original view.

### **Smart Default Logic and Multiplier Selection**

To maintain the application's goal of simplicity, the upscaling interface provides "smart defaults" that automatically select the optimal model and multiplier based on the characteristics of the source image.25

| Feature | Real-ESRGAN (General) | Real-ESRGAN (Anime) | Waifu2x (ncnn) |
| :---- | :---- | :---- | :---- |
| **Optimized For** | Real-world photos, complex textures.31 | Digital art, clean lines, anime.25 | Line art, screenshots, UI elements.31 |
| **Native Multipliers** | 4x (Standard).32 | 4x (Compact).32 | 2x (Recursive support).35 |
| **Hardware Requirement** | Vulkan-compatible GPU.26 | Low VRAM (\~1GB+).25 | Ultra-low VRAM (\<1GB).31 |

The application calculates the target scale using a resolution-aware formula:

$$TargetScale \= \\max(2, \\min(4, \\text{ceil}(3840 / \\text{SourceWidth})))$$  
This targets an output width of approximately 3840 pixels (standard 4K UHD), ensuring the resulting image is optimized for modern displays without the unnecessary bloat of higher multipliers.30

## **Metadata Overlays and Tagging Standards**

A core requirement of the application is the display of detailed metadata to help the user understand the quality and provenance of their image library. The implementation leverages GExiv2, a GObject-based wrapper around the industry-standard Exiv2 library.39

### **Standardized Metadata Handling**

For library organization, the application prioritizes the Extensible Metadata Platform (XMP) standard. XMP allows for the storage of custom tags, ratings, and hierarchical keywords recognized by other professional tools like Adobe Lightroom or Digikam.42 The application provides a tagging UI where users apply keywords persisted into the image's XMP block or an optional "sidecar" mode (.xmp file) to preserve the original file's hash.42

### **Floating Metadata Interface**

Following the reference design, metadata is displayed via a floating overlay on the primary image preview. This is implemented using the GtkOverlay widget. The base child is a GtkPicture, while the metadata chip itself is an AdwBin containing technical "Quality Indicators" such as file size, compression type, and megapixel count.43

## **Flatpak-First Architecture and Sandboxing**

The application is designed from the ground up to be distributed as a Flatpak, ensuring consistent dependencies for NCNN and Vulkan.9

### **Sandbox Permissions and Portals**

The application adheres to the principle of least privilege using XDG Portals:47

1. **File Selection:** Uses GtkFileChooserNative. Once a user selects a folder, the sandbox is granted a temporary handle, ensuring the app cannot see unrelated home directory data.47  
2. **GPU Acceleration:** Manifest includes \--device=dri, allowing the NCNN upscaler and GTK4 renderer to communicate directly with the graphics driver.51  
3. **IPC and Shared Memory:** Utilizes \--share=ipc for efficient buffer sharing with NCNN subprocesses.51

## **Detailed Implementation Roadmap for Agentic Execution**

The following roadmap provides discrete, verifiable steps for building the application.

### **Phase 1: Core Foundation and Data Models**

* **Objective:** Establish Rust project structure and define primary GObjects.  
* **Key Tasks:** Initialize project with gtk4-rs, libadwaita-rs, and rexiv2.3 Implement ImageEntry and LibraryManager.20

### **Phase 2: User Interface Shell and Shortcuts**

* **Objective:** Construct adaptive triple-pane layout and interaction system.  
* **Key Tasks:** Define UI in Blueprint file.13 Set up AdwBreakpoint for 1200px/800px.11 Implement GtkShortcutController for zooming (Ctrl+Wheel) and navigation (Alt+Arrows).2

### **Phase 3: High-Performance Filmstrip and Loading**

* **Objective:** Implement recycled list view and async thumbnailing.  
* **Key Tasks:** Bind GtkListView to GListStore.19 Implement factory callbacks and background worker.2

### **Phase 4: Image Preview and Metadata Overlays**

* **Objective:** Build primary viewer and Technical Metadata display.  
* **Key Tasks:** Utilize GtkOverlay for the metadata chip.55 Integrate GExiv2 for EXIF/XMP extraction.57

### **Phase 5: AI Upscaling and Comparison UX**

* **Objective:** Connect NCNN Vulkan backend and implement comparison slider.  
* **Key Tasks:**  
  * Invoke realesrgan-ncnn-vulkan via GSubprocess with .osd progress bar.  
  * Implement **interactive split-view** comparison using GSK clipping nodes and a draggable divider.  
  * Add resolution-aware multiplier selection.30

### **Phase 6: Packaging and Optimization**

* **Objective:** Finalize Flatpak manifest and performance tuning.  
* **Key Tasks:** Bundle NCNN binaries and models in Flatpak.9 Apply Adwaita dark CSS.13

## **Quantitative Quality Verification Metrics**

| Metric | Target | Verification Method |
| :---- | :---- | :---- |
| **Initial Startup Time** | \< 500 ms.9 | flatpak run \--verbose imglib profiling. |
| **Scrolling Performance** | Stable 60 FPS.1 | GTK\_DEBUG=interactive frame clock monitor. |
| **VRAM Peak (4k Upscale)** | \< 1.5 GB.25 | nvidia-smi monitor. |

#### **Works cited**

1. Why GNOME Replaced Eye of GNOME with Loupe as the Default Image Viewer, accessed April 4, 2026, [https://www.linuxjournal.com/content/why-gnome-replaced-eye-gnome-loupe-default-image-viewer](https://www.linuxjournal.com/content/why-gnome-replaced-eye-gnome-loupe-default-image-viewer)  
2. What I learned using Claude Sonnet to migrate Python to Rust \- InfoWorld, accessed April 4, 2026, [https://www.infoworld.com/article/4135218/what-i-learned-using-claude-sonnet-to-migrate-python-to-rust.html](https://www.infoworld.com/article/4135218/what-i-learned-using-claude-sonnet-to-migrate-python-to-rust.html)  
3. Coding Rust With Claude Code and Codex \- HackerNoon, accessed April 4, 2026, [https://hackernoon.com/coding-rust-with-claude-code-and-codex](https://hackernoon.com/coding-rust-with-claude-code-and-codex)  
4. Rust vs. Python: Finding the Right Balance Between Speed and Simplicity, accessed April 4, 2026, [https://blog.jetbrains.com/rust/2025/11/10/rust-vs-python-finding-the-right-balance-between-speed-and-simplicity/](https://blog.jetbrains.com/rust/2025/11/10/rust-vs-python-finding-the-right-balance-between-speed-and-simplicity/)  
5. Why I Chose Rust Over Python for Production AI Systems \- DEV Community, accessed April 4, 2026, [https://dev.to/mayu2008/why-i-chose-rust-over-python-for-production-ai-systems-5gim](https://dev.to/mayu2008/why-i-chose-rust-over-python-for-production-ai-systems-5gim)  
6. Should Developers Switch from Rust to Python for AI in 2025? | Practical Guide, accessed April 4, 2026, [https://aarambhdevhub.medium.com/should-developers-switch-from-rust-to-python-for-ai-in-2025-practical-guide-3d767f0c264f](https://aarambhdevhub.medium.com/should-developers-switch-from-rust-to-python-for-ai-in-2025-practical-guide-3d767f0c264f)  
7. Rust vs Python: Which Is Better for AI Performance? \- CodingCops, accessed April 4, 2026, [https://codingcops.com/rust-vs-python-ai-performance/](https://codingcops.com/rust-vs-python-ai-performance/)  
8. What GTK+ sub-process/threading/program execution/etc should I use if I want to launch a program from a GTK+ app? \- Stack Overflow, accessed April 4, 2026, [https://stackoverflow.com/questions/68250487/what-gtk-sub-process-threading-program-execution-etc-should-i-use-if-i-want-to](https://stackoverflow.com/questions/68250487/what-gtk-sub-process-threading-program-execution-etc-should-i-use-if-i-want-to)  
9. How to Build Flatpak Applications on Ubuntu \- OneUptime, accessed April 4, 2026, [https://oneuptime.com/blog/post/2026-03-02-how-to-build-flatpak-applications-on-ubuntu/view](https://oneuptime.com/blog/post/2026-03-02-how-to-build-flatpak-applications-on-ubuntu/view)  
10. Combining Rust and Python for High-Performance AI Systems \- The New Stack, accessed April 4, 2026, [https://thenewstack.io/combining-rust-and-python-for-high-performance-ai-systems/](https://thenewstack.io/combining-rust-and-python-for-high-performance-ai-systems/)  
11. Adw – 1: Adaptive Layouts, accessed April 4, 2026, [https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1.5/adaptive-layouts.html](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1.5/adaptive-layouts.html)  
12. GNOME Human Interface Guidelines, accessed April 4, 2026, [https://developer.gnome.org/hig/](https://developer.gnome.org/hig/)  
13. Adw.NavigationSplitView, accessed April 4, 2026, [https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1.2/class.NavigationSplitView.html](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1.2/class.NavigationSplitView.html)  
14. Libadwaita 1.6 \- GNOME Blogs, accessed April 4, 2026, [https://blogs.gnome.org/alicem/2024/09/13/libadwaita-1-6/](https://blogs.gnome.org/alicem/2024/09/13/libadwaita-1-6/)  
15. Adw.OverlaySplitView, accessed April 4, 2026, [https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1.4/class.OverlaySplitView.html](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1.4/class.OverlaySplitView.html)  
16. How achieve this UI ? I can't figure out how make Adwaita work like GtkStackSidebar : r/gnome \- Reddit, accessed April 4, 2026, [https://www.reddit.com/r/gnome/comments/1evyoco/how\_achieve\_this\_ui\_i\_cant\_figure\_out\_how\_make/](https://www.reddit.com/r/gnome/comments/1evyoco/how_achieve_this_ui_i_cant_figure_out_how_make/)  
17. New GNOME Human Interface Guidelines now official – and obviously some people hate it, accessed April 4, 2026, [https://www.theregister.com/2021/08/09/gnome\_human\_interface\_guidelines/](https://www.theregister.com/2021/08/09/gnome_human_interface_guidelines/)  
18. Adw.NavigationSplitView – libadwaita-1 \- Valadoc, accessed April 4, 2026, [https://valadoc.org/libadwaita-1/Adw.NavigationSplitView.html](https://valadoc.org/libadwaita-1/Adw.NavigationSplitView.html)  
19. A primer on GtkListView – GTK Development Blog \- GNOME Blogs, accessed April 4, 2026, [https://blogs.gnome.org/gtk/2020/09/05/a-primer-on-gtklistview/](https://blogs.gnome.org/gtk/2020/09/05/a-primer-on-gtklistview/)  
20. GtkListView \- GTK 4 tutorial, accessed April 4, 2026, [https://toshiocp.github.io/Gtk4-tutorial/sec29.html](https://toshiocp.github.io/Gtk4-tutorial/sec29.html)  
21. GTK4 efficient ListView \- Development \- GNOME Discourse, accessed April 4, 2026, [https://discourse.gnome.org/t/gtk4-efficient-listview/31450](https://discourse.gnome.org/t/gtk4-efficient-listview/31450)  
22. GTK 4 ListView with .NET \- 4 \- DEV Community, accessed April 4, 2026, [https://dev.to/kashifsoofi/gtk-4-listview-with-net-8-219g](https://dev.to/kashifsoofi/gtk-4-listview-with-net-8-219g)  
23. Gtk.ListView – gtk4 \- Valadoc.org, accessed April 4, 2026, [https://valadoc.org/gtk4/Gtk.ListView.html](https://valadoc.org/gtk4/Gtk.ListView.html)  
24. Gtk.ListView, accessed April 4, 2026, [https://docs.gtk.org/gtk4/class.ListView.html](https://docs.gtk.org/gtk4/class.ListView.html)  
25. NCNN Executable \- Real-ESRGAN \- Mintlify, accessed April 4, 2026, [https://mintlify.com/xinntao/Real-ESRGAN/guides/ncnn-executable](https://mintlify.com/xinntao/Real-ESRGAN/guides/ncnn-executable)  
26. Real-ESRGAN ncnn Vulkan download | SourceForge.net, accessed April 4, 2026, [https://sourceforge.net/projects/real-esrgan-ncnn-vulkan.mirror/](https://sourceforge.net/projects/real-esrgan-ncnn-vulkan.mirror/)  
27. xinntao/Real-ESRGAN-ncnn-vulkan \- GitHub, accessed April 4, 2026, [https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan](https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan)  
28. Gio.Subprocess, accessed April 4, 2026, [https://docs.gtk.org/gio/class.Subprocess.html](https://docs.gtk.org/gio/class.Subprocess.html)  
29. Real-ESRGAN & the Super-Resolution Revolution | by Johnstri A. S. | Medium, accessed April 4, 2026, [https://medium.com/@johnstri/leveraging-real-esrgan-for-high-fidelity-image-reconstruction-4aca8be65942](https://medium.com/@johnstri/leveraging-real-esrgan-for-high-fidelity-image-reconstruction-4aca8be65942)  
30. How to Increase Resolution of an Image With AI: Complete Guide \- Artsmart.ai, accessed April 4, 2026, [https://artsmart.ai/blog/how-to-increase-resolution-of-an-image-with-ai/](https://artsmart.ai/blog/how-to-increase-resolution-of-an-image-with-ai/)  
31. Improve the Quality of Enlarged Images with These Photo Techniques \- LifeTips, accessed April 4, 2026, [https://lifetips.alibaba.com/tech-efficiency/improve-the-quality-of-enlarged-images-with-these-photo](https://lifetips.alibaba.com/tech-efficiency/improve-the-quality-of-enlarged-images-with-these-photo)  
32. Real-ESRGAN \- CodeSandbox, accessed April 4, 2026, [http://codesandbox.io/p/github/NightmareAI/Real-ESRGAN](http://codesandbox.io/p/github/NightmareAI/Real-ESRGAN)  
33. waifu2x Test on 28 Days Later \- Real-CUGAN-NCNN-Vulkan (48 Minuets on RTX 3060), accessed April 4, 2026, [https://www.reddit.com/r/waifu2x/comments/1fu9bhb/waifu2x\_test\_on\_28\_days\_later\_realcuganncnnvulkan/](https://www.reddit.com/r/waifu2x/comments/1fu9bhb/waifu2x_test_on_28_days_later_realcuganncnnvulkan/)  
34. Real-ESRGAN aims at developing Practical Algorithms for General Image/Video Restoration. \- GitHub, accessed April 4, 2026, [https://github.com/xinntao/real-esrgan](https://github.com/xinntao/real-esrgan)  
35. waifu2x-ncnn-vulkan-python/Docs.md at master \- GitHub, accessed April 4, 2026, [https://github.com/media2x/waifu2x-ncnn-vulkan-python/blob/master/Docs.md](https://github.com/media2x/waifu2x-ncnn-vulkan-python/blob/master/Docs.md)  
36. Waifu2x \- Types, what differences ? : r/Dandere2x \- Reddit, accessed April 4, 2026, [https://www.reddit.com/r/Dandere2x/comments/ljdgt0/waifu2x\_types\_what\_differences/](https://www.reddit.com/r/Dandere2x/comments/ljdgt0/waifu2x_types_what_differences/)  
37. AI image and video upscaling to 4K in 1 workflow | LetsEnhance 2026, accessed April 4, 2026, [https://letsenhance.io/blog/all/image-video-upscaling/](https://letsenhance.io/blog/all/image-video-upscaling/)  
38. How to Upscale 1080p to 4K in Only a Few Steps \- Boris FX, accessed April 4, 2026, [https://borisfx.com/blog/how-to-upscale-1080p-to-4k-in-only-a-few-steps/](https://borisfx.com/blog/how-to-upscale-1080p-to-4k-in-only-a-few-steps/)  
39. index.htm – gexiv2 \- Valadoc.org, accessed April 4, 2026, [https://valadoc.org/gexiv2/index.htm](https://valadoc.org/gexiv2/index.htm)  
40. Projects/gexiv2 – GNOME Wiki Archive, accessed April 4, 2026, [https://wiki.gnome.org/Projects/gexiv2](https://wiki.gnome.org/Projects/gexiv2)  
41. GIMP Gets Good Metadata Support Via Exiv2 \- Phoronix, accessed April 4, 2026, [https://www.phoronix.com/news/MTUwNjY](https://www.phoronix.com/news/MTUwNjY)  
42. Extensible Metadata Platform \- Wikipedia, accessed April 4, 2026, [https://en.wikipedia.org/wiki/Extensible\_Metadata\_Platform](https://en.wikipedia.org/wiki/Extensible_Metadata_Platform)  
43. Photo Metadata Standards: IPTC, EXIF, and XMP \- DEV Community, accessed April 4, 2026, [https://dev.to/maryalice/photo-metadata-standards-iptc-exif-and-xmp-4n5d](https://dev.to/maryalice/photo-metadata-standards-iptc-exif-and-xmp-4n5d)  
44. EXIF/IPTC metadata or XMP sidecar files ? : r/photography \- Reddit, accessed April 4, 2026, [https://www.reddit.com/r/photography/comments/1mqay9o/exifiptc\_metadata\_or\_xmp\_sidecar\_files/](https://www.reddit.com/r/photography/comments/1mqay9o/exifiptc_metadata_or_xmp_sidecar_files/)  
45. Gtk.Image \- GTK Documentation, accessed April 4, 2026, [https://docs.gtk.org/gtk4/class.Image.html](https://docs.gtk.org/gtk4/class.Image.html)  
46. Overlay in gtk4 \- Rust, accessed April 4, 2026, [https://gtk-rs.org/gtk4-rs/git/docs/gtk4/struct.Overlay.html](https://gtk-rs.org/gtk4-rs/git/docs/gtk4/struct.Overlay.html)  
47. When Flatpak's Sandbox Cracks: Real‑Life Security Issues Beyond the Ideal | Linux Journal, accessed April 4, 2026, [https://www.linuxjournal.com/content/when-flatpaks-sandbox-cracks-real-life-security-issues-beyond-ideal](https://www.linuxjournal.com/content/when-flatpaks-sandbox-cracks-real-life-security-issues-beyond-ideal)  
48. How to Manage Flatpak Permissions and Sandbox Overrides on RHEL \- OneUptime, accessed April 4, 2026, [https://oneuptime.com/blog/post/2026-03-04-manage-flatpak-permissions-sandbox-overrides-rhel/view](https://oneuptime.com/blog/post/2026-03-04-manage-flatpak-permissions-sandbox-overrides-rhel/view)  
49. FileChooser in gtk4 \- Rust, accessed April 4, 2026, [https://gtk-rs.org/gtk4-rs/git/docs/gtk4/struct.FileChooser.html](https://gtk-rs.org/gtk4-rs/git/docs/gtk4/struct.FileChooser.html)  
50. Flatpaks need the ability to request user permissions like iOS/Android : r/linux \- Reddit, accessed April 4, 2026, [https://www.reddit.com/r/linux/comments/1l9jl1d/flatpaks\_need\_the\_ability\_to\_request\_user/](https://www.reddit.com/r/linux/comments/1l9jl1d/flatpaks_need_the_ability_to_request_user/)  
51. Sandbox Permissions \- Flatpak documentation, accessed April 4, 2026, [https://docs.flatpak.org/en/latest/sandbox-permissions.html](https://docs.flatpak.org/en/latest/sandbox-permissions.html)  
52. Manifests \- Flatpak documentation, accessed April 4, 2026, [https://docs.flatpak.org/en/latest/manifests.html](https://docs.flatpak.org/en/latest/manifests.html)  
53. Libadwaita \- GUI development with Rust and GTK 4, accessed April 4, 2026, [https://gtk-rs.org/gtk4-rs/git/book/libadwaita.html](https://gtk-rs.org/gtk4-rs/git/book/libadwaita.html)  
54. felixc/rexiv2: Rust library for read/write access to media-file metadata (Exif, XMP, and IPTC), accessed April 4, 2026, [https://github.com/felixc/rexiv2](https://github.com/felixc/rexiv2)  
55. Gtk.Overlay – gtk4 \- Valadoc.org, accessed April 4, 2026, [https://valadoc.org/gtk4/Gtk.Overlay.html](https://valadoc.org/gtk4/Gtk.Overlay.html)  
56. Gtk.Overlay, accessed April 4, 2026, [https://docs.gtk.org/gtk4/class.Overlay.html](https://docs.gtk.org/gtk4/class.Overlay.html)  
57. What is image metadata (EXIF, IPTC, XMP)? \- ImageRanger, accessed April 4, 2026, [https://imageranger.com/tips/image\_metadata\_formats/](https://imageranger.com/tips/image_metadata_formats/)  
58. What are image metadata (EXIF, IPTC, XMP)? \- NeededApps, accessed April 4, 2026, [https://neededapps.com/tutorials/what-are-image-metadata-exif-iptc-xmp/](https://neededapps.com/tutorials/what-are-image-metadata-exif-iptc-xmp/)  
59. flathub/com.github.nihui.waifu2x-ncnn-vulkan, accessed April 4, 2026, [https://github.com/flathub/com.github.nihui.waifu2x-ncnn-vulkan](https://github.com/flathub/com.github.nihui.waifu2x-ncnn-vulkan)  
60. Simple hack to tinting/theming Libadwaita GTK4 apps in KDE Plasma \- Tips and Tricks, accessed April 4, 2026, [https://discuss.kde.org/t/simple-hack-to-tinting-theming-libadwaita-gtk4-apps-in-kde-plasma/29444](https://discuss.kde.org/t/simple-hack-to-tinting-theming-libadwaita-gtk4-apps-in-kde-plasma/29444)  
61. Human Interface Guidelines \- iFixit, accessed April 4, 2026, [https://documents.cdn.ifixit.com/F2RgMMmS1FUKqYHx.pdf](https://documents.cdn.ifixit.com/F2RgMMmS1FUKqYHx.pdf)  
62. PSA: If GTK 4 apps feel slower than their GTK 3 version (e.g. Nautilus), try changing GTK's renderer to cairo : r/gnome \- Reddit, accessed April 4, 2026, [https://www.reddit.com/r/gnome/comments/ywuof9/psa\_if\_gtk\_4\_apps\_feel\_slower\_than\_their\_gtk\_3/](https://www.reddit.com/r/gnome/comments/ywuof9/psa_if_gtk_4_apps_feel_slower_than_their_gtk_3/)