import Foundation
@main struct VerifyPets {
    static func main() throws {
        for (row,count,ms) in [(0,6,900),(1,8,150),(3,4,180),(4,5,180),(5,8,180),(6,6,260),(7,6,150),(8,6,220)] {
            let pet=PetState(behavior:"fixture",label:"Fixture",row:row,frames:count,frameMs:Double(ms),key:"fixture")
            precondition(pet.frame(elapsed:0,reduced:false)==row*8)
            precondition(pet.frame(elapsed:100,reduced:true)==row*8)
            precondition(pet.frame(elapsed:100,reduced:false)<6)
        }
        print("Pet animation bounds, reduced motion and settle timing passed")
    }
}
