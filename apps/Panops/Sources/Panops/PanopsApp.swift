import SwiftUI

@main
struct PanopsApp: App {
    var body: some Scene {
        WindowGroup("Panops") {
            VStack(spacing: 16) {
                Text("Panops")
                    .font(.largeTitle)
                Text("Walking skeleton — slice 09")
                    .foregroundStyle(.secondary)
            }
            .frame(minWidth: 480, minHeight: 320)
            .padding()
        }
        .windowResizability(.contentSize)
    }
}
