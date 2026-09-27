use demo::DemoApp;

fn main() {
    let root = web_sys::window()
        .expect("window")
        .document()
        .expect("document")
        .get_element_by_id("app")
        .expect("#app is in index.html");
    yew::Renderer::<DemoApp>::with_root(root).render();
}
